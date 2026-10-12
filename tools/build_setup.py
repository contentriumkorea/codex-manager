"""Compile the full Windows installer from the same verified updater payload."""
import argparse
import hashlib
import json
import subprocess
import sys
from pathlib import Path

ROOT=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT/'src'))
from project_manager.version import VERSION
from project_manager.files import digest


def build_setup(compiler,dist,output,test=False):
    dist=dist.resolve();output=output.resolve();output.mkdir(parents=True,exist_ok=True)
    manifest=json.loads((dist/'install-manifest.json').read_text(encoding='utf-8'))
    if manifest['version']!=VERSION:raise ValueError('Build version does not match installer version.')
    files={p.relative_to(dist).as_posix() for p in dist.rglob('*') if p.is_file()}
    if files!=set(manifest['files'])|{'install-manifest.json'}:raise ValueError('Installer source contains untracked files.')
    if any(digest(dist/p)!=h for p,h in manifest['files'].items()):raise ValueError('Installer source hash mismatch.')
    args=[str(compiler),'/Qp','/DAppVersion='+VERSION,'/DSourceDir='+str(dist),'/DOutputDir='+str(output)]
    if test:args+=['/DAppId=Contentrium.CodexManager.QA','/DAppTitle=Codex Manager QA']
    subprocess.run(args+[str(ROOT/'packaging/codex-manager.iss')],check=True)
    setup=output/'Codex-Manager-Setup.exe'
    if test:
        (output/'setup-qa.json').write_text(json.dumps({'test':True,'sha256':digest(setup)}),encoding='utf-8')
    else:
        names=('Codex-Manager-Setup.exe','Codex-Manager-Windows-x64.zip','update.json')
        (output/'SHA256SUMS.txt').write_text(''.join(digest(output/name)+'  '+name+'\n' for name in names),encoding='utf-8')
    print(json.dumps({'setup':str(setup),'size':setup.stat().st_size,'sha256':digest(setup),'test':test}))


if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('--compiler',type=Path,required=True);parser.add_argument('--dist',type=Path,required=True);parser.add_argument('--output',type=Path,required=True);parser.add_argument('--test',action='store_true')
    args=parser.parse_args();build_setup(args.compiler,args.dist,args.output,args.test)
