"""Build the fixed-name GitHub updater assets."""
import argparse,hashlib,json,sys,zipfile
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'src'))
from project_manager.version import VERSION,ASSET_NAME,EXECUTABLE
from project_manager.bundles import write_json


def build(dist,output):
    if output.exists(): raise FileExistsError('이미 만들어진 릴리스 폴더입니다: '+str(output))
    if not (dist/EXECUTABLE).is_file() or not (dist/'_internal').is_dir(): raise ValueError('빌드된 실행 파일이 없습니다.')
    paths=sorted(p for p in dist.rglob('*') if p.is_file() and p.name!='install-manifest.json')
    if any(p.suffix in ('.sqlite','.jsonl') or p.name in ('auth.json','.env','.codex-global-state.json') for p in paths): raise ValueError('사용자 기록이 배포에 들어갈 수 없습니다.')
    manifest={'version':VERSION,'files':{p.relative_to(dist).as_posix():hashlib.sha256(p.read_bytes()).hexdigest() for p in paths}}
    write_json(dist/'install-manifest.json',manifest);output.mkdir(parents=True)
    archive=output/ASSET_NAME
    with zipfile.ZipFile(archive,'x',zipfile.ZIP_DEFLATED,compresslevel=6) as z:
        for p in [*paths,dist/'install-manifest.json']: z.write(p,'CodexManager/'+p.relative_to(dist).as_posix())
    with zipfile.ZipFile(archive) as z:
        if z.testzip(): raise ValueError('압축파일 CRC 검증 실패')
    metadata={'version':VERSION,'asset':ASSET_NAME,'size':archive.stat().st_size,'sha256':hashlib.sha256(archive.read_bytes()).hexdigest()}
    write_json(output/'update.json',metadata)
    checksums=''.join(hashlib.sha256((output/name).read_bytes()).hexdigest()+'  '+name+'\n' for name in (ASSET_NAME,'update.json'))
    (output/'SHA256SUMS.txt').write_text(checksums,encoding='utf-8')
    print(json.dumps(metadata));return archive


if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('--dist',type=Path,default=Path('dist/CodexManager'));parser.add_argument('--output',type=Path,default=Path('releases')/('v'+VERSION))
    args=parser.parse_args();build(args.dist,args.output)
