"""Run a disposable cross-home fixture workflow, reporting external checks separately."""
import argparse
import json
import sys
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'src'))
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'tests/integration'))
from project_manager.codex.adapter import CodexAdapter
from project_manager.operations import transfer_project,import_bundle
from project_manager.bundles import export_project,write_json
from project_manager.restore_proof import verify_restored_bundle
from project_manager.cleanup import cleanup_export
from project_manager.journal import Journal
from test_operations import make_project
from test_codex_portability import append_continuation


def main():
    parser=argparse.ArgumentParser();parser.add_argument('--isolated-root',type=Path,required=True);parser.add_argument('--report',type=Path,required=True)
    parser.add_argument('--fixture-only',action='store_true',help='Only require disposable local workflow checks')
    args=parser.parse_args();root=args.isolated_root.resolve()
    allowed=(Path(__file__).resolve().parents[1]/'.test-artifacts').resolve()
    if not root.is_relative_to(allowed) or root==allowed or root.exists():
        raise SystemExit('새로운 .test-artifacts 하위 폴더만 검증 대상으로 사용할 수 있습니다.')
    root.mkdir(parents=True)
    import faulthandler
    faulthandler.dump_traceback_later(50,repeat=True)
    print('Creating isolated fixtures',flush=True)
    source=CodexAdapter(root/'source-home',isolated=True)
    pa,tid=make_project(source,root/'A','A');pb,_=make_project(source,root/'B','B')
    journal=Journal(root/'source.sqlite')
    print('Merge',flush=True)
    merged=transfer_project(source,pa,{str(pa.roots[0]):root/'B'/'A'},journal,root/'recovery',target=pb,clean=True)
    if merged.state!='completed': raise RuntimeError(str(merged.errors))
    p=next(p for p in source.snapshot().projects if p.id==pb.id)
    bundle=root/'bundle';export_project(p,source.snapshot(),bundle)
    print('Import',flush=True)
    target=CodexAdapter(root/'target-home',isolated=True)
    imported=import_bundle(bundle,{'root-01':root/'other-user'/'files'},target,Journal(root/'import.sqlite'))
    if imported.state!='completed': raise RuntimeError(str(imported.errors))
    t=next(t for t in target.snapshot().conversations if t.id==tid)
    append_continuation(t.rollout)
    print('Restore proof',flush=True)
    proof=verify_restored_bundle(bundle,target)
    if not proof.ok: raise RuntimeError(str(proof.errors))
    cleaned=cleanup_export(p,bundle,source,journal)
    if cleaned.state!='completed': raise RuntimeError(str(cleaned.errors))
    write_json(args.report,{'fixture_workflow_pass':True,'full_acceptance_pass':False,'server':target.server_version(),
               'simulated_turn':True,'actual_account_continuation':False,'physical_second_pc':False,'mobile':False,
               'original_test_a_b_unchanged':True})
    print('Fixture workflow PASS; actual account / second physical PC / mobile checks remain pending.')
    return 0 if args.fixture_only else 2


if __name__=='__main__': raise SystemExit(main())
