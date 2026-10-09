"""Regression boundaries between display inference and destructive operations."""
import json
import uuid
from dataclasses import replace

import pytest

from project_manager.bundles import export_project, load_manifest, verify_bundle, write_json
from project_manager.cleanup import cleanup_export
from project_manager.files import digest
from project_manager.grouping import display_membership
from project_manager.journal import Journal
from project_manager.models import Project, Snapshot
from project_manager.operations import import_bundle, transfer_project
from test_review_regressions import Adapter, fixture


@pytest.mark.parametrize('operation', ['move', 'cleanup'])
def test_new_unassigned_chat_blocks_source_removal_before_any_mutation(tmp_path, operation):
    adapter, project, original, bundle = fixture(tmp_path)
    stale_snapshot = adapter.snapshot()
    assert not display_membership(stale_snapshot, {}).inferred
    added = replace(original, id=str(uuid.uuid4()), project_id=None, parent_id=None)
    adapter.threads.append(added)
    before = adapter.snapshot()
    journal = Journal(tmp_path / 'journal.sqlite')
    destination = tmp_path / 'moved'

    with pytest.raises(ValueError, match='연결'):
        if operation == 'move':
            transfer_project(adapter, project, {str(project.roots[0]): destination},
                             journal, tmp_path / 'recovery', clean=True)
        else:
            cleanup_export(project, bundle, adapter, journal)

    assert adapter.snapshot() == before
    assert not adapter.deleted and not journal.pending()
    assert (project.roots[0] / 'file.txt').read_text() == 'original'
    assert added.cwd.exists() and not destination.exists()
    assert not (tmp_path / 'recovery').exists()


def external_child_bundle(tmp_path, external_kind):
    adapter, project, parent, _ = fixture(tmp_path)
    outside = tmp_path / 'child-workspace'
    outside.mkdir()
    (outside / 'not-captured.txt').write_text('child work')
    cwd = outside if external_kind == 'cwd' else parent.cwd
    runtime = (outside,) if external_kind == 'runtime' else (cwd,)
    tid = str(uuid.uuid4())
    raw = tmp_path / 'child.jsonl'
    raw.write_text(json.dumps({'type': 'session_meta', 'payload': {
        'id': tid, 'cwd': str(cwd), 'runtime_workspace_roots': [str(p) for p in runtime]
    }}) + '\n', encoding='utf-8')
    child = replace(parent, id=tid, project_id=None, parent_id=parent.id,
                    cwd=cwd, runtime_roots=runtime, rollout=raw, internal=True)
    adapter.threads.append(child)
    snapshot = adapter.snapshot()
    membership = display_membership(snapshot, {})
    assert membership.inferred[child.id] == project.id
    bundle = tmp_path / 'with-child'
    assert export_project(project, snapshot, bundle, folder_memberships=membership.inferred).ok
    return bundle, child, outside


@pytest.mark.parametrize('external_kind', ['cwd', 'runtime'])
def test_inferred_child_external_workspace_is_reported_before_import_mutates(tmp_path, external_kind):
    bundle, child, outside = external_child_bundle(tmp_path, external_kind)
    manifest = load_manifest(bundle)
    assert child.id in manifest['folder_grouped_threads']
    assert next(t for t in manifest['conversations'] if t['id'] == child.id)['project_id'] is None
    assert any(d['kind'] == 'external_workspace' and d['thread_id'] == child.id
               and d['resolved'] is False for d in manifest['dependencies'])
    assert verify_bundle(bundle).ok
    target = Adapter(tmp_path / 'target-home')
    journal = Journal(tmp_path / 'import.sqlite')
    destination = tmp_path / 'restored'
    with pytest.raises(ValueError):
        import_bundle(bundle, {'root-01': destination}, target, journal)
    assert not target.projects and not target.threads and not journal.pending()
    assert not destination.exists()
    assert (outside / 'not-captured.txt').read_text() == 'child work'


@pytest.mark.parametrize('inherited_parent', [False, True])
@pytest.mark.parametrize('reverse', [False, True])
def test_conflicting_parent_and_folder_stays_unassigned_regardless_of_order(tmp_path, inherited_parent, reverse):
    _, first, template, _ = fixture(tmp_path)
    second = Project('second', 'Second', (tmp_path / 'second',))
    parent = replace(template, id='parent', project_id=first.id)
    conversations = [parent]
    if inherited_parent:
        grandparent = replace(template, id='grandparent', project_id=first.id)
        parent = replace(parent, project_id=None, cwd=tmp_path / 'outside', parent_id=grandparent.id)
        conversations = [grandparent, parent]
    child = replace(template, id='child', project_id=None, cwd=second.roots[0], parent_id=parent.id)
    conversations.append(child)
    if reverse:
        conversations.reverse()
    membership = display_membership(Snapshot((first, second), tuple(conversations), ''), {})
    assert membership.projects[parent.id] == first.id
    assert membership.projects[child.id] is None
    assert membership.reasons[child.id] == 'ambiguous'
    assert child.id not in membership.inferred


@pytest.mark.parametrize('external_kind', ['cwd', 'runtime'])
def test_checksum_valid_omitted_dependency_still_fails_mapping_before_any_import_mutation(tmp_path, external_kind):
    bundle, _, _ = external_child_bundle(tmp_path, external_kind)
    manifest = load_manifest(bundle)
    manifest['dependencies'] = []
    write_json(bundle / 'manifest.json', manifest)
    checksums = bundle / 'checksums.jsonl'
    entries = [json.loads(line) for line in checksums.read_text(encoding='utf-8').splitlines()]
    for entry in entries:
        if entry['path'] == 'manifest.json':
            entry['size'] = (bundle / 'manifest.json').stat().st_size
            entry['sha256'] = digest(bundle / 'manifest.json')
    checksums.write_text(''.join(json.dumps(e) + '\n' for e in entries), encoding='utf-8')
    assert verify_bundle(bundle).ok

    target = Adapter(tmp_path / 'target-home')
    journal = Journal(tmp_path / 'import.sqlite')
    destination = tmp_path / 'restored'
    with pytest.raises(ValueError, match='작업 경로'):
        import_bundle(bundle, {'root-01': destination}, target, journal)
    assert not destination.exists()
    assert not target.projects and not target.threads and not journal.pending()
