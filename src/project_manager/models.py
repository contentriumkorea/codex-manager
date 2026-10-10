from dataclasses import dataclass, field
from pathlib import Path


@dataclass(frozen=True)
class Project:
    id: str
    name: str
    roots: tuple[Path,...]
    legacy_ids: tuple[str,...] = ()


@dataclass(frozen=True)
class Conversation:
    id: str
    project_id: str|None
    cwd: Path
    runtime_roots: tuple[Path,...]
    archived: bool
    parent_id: str|None
    revision: str
    running: bool
    title: str = ''
    rollout: Path|None = None
    updated_at: int = 0
    attachments: tuple[dict,...] = ()
    internal: bool = False


@dataclass(frozen=True)
class Snapshot:
    projects: tuple[Project,...]
    conversations: tuple[Conversation,...]
    revision: str


@dataclass(frozen=True)
class Capabilities:
    server_version: str
    supported: frozenset[str]
    portability_verified: bool
    reasons: tuple[str,...]


@dataclass(frozen=True)
class FileEntry:
    root_id: str
    relative_path: str
    size: int
    sha256: str|None
    file_id: str
    mtime_ns: int


@dataclass(frozen=True)
class Inventory:
    entries: tuple[FileEntry,...]
    blockers: tuple[str,...]
    logical_bytes: int
    allocated_bytes: int|None
    directories: tuple[tuple[str,str],...] = ()


@dataclass(frozen=True)
class Verification:
    ok: bool
    errors: tuple[str,...] = ()
    evidence_path: Path|None = None


@dataclass(frozen=True)
class Request:
    kind: str
    project_ids: tuple[str,...]
    conversation_ids: tuple[str,...] = ()
    destinations: dict[str,Path] = field(default_factory=dict)
    target_project_id: str|None = None
    conflict_choices: dict[str,str] = field(default_factory=dict)


@dataclass(frozen=True)
class OperationPlan:
    id: str
    request: Request
    snapshot_revision: str
    inventory: Inventory
    blockers: tuple[str,...]
    affected_thread_ids: tuple[str,...]


@dataclass(frozen=True)
class OperationResult:
    state: str
    file_status: str
    codex_status: str
    cleanup_status: str
    mobile_status: str = '모바일 미확인'
    errors: tuple[str,...] = ()
    report_id: str|None = None
