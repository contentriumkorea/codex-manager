"""Build the fixed-name GitHub updater assets."""
import argparse,hashlib,json,sys,zipfile
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'src'))
sys.path.insert(0,str(Path(__file__).resolve().parent))
from project_manager.version import VERSION,ASSET_NAME,EXECUTABLE
from project_manager.bundles import write_json
from project_manager.files import linked
from release_payload import public_path,validate_archive


def build(dist,output):
    if output.exists(): raise FileExistsError('이미 만들어진 릴리스 폴더입니다: '+str(output))
    if not (dist/EXECUTABLE).is_file() or not (dist/'_internal').is_dir(): raise ValueError('빌드된 실행 파일이 없습니다.')
    paths=[]
    def collect(folder):
        if linked(folder): raise ValueError('Linked folders cannot be published.')
        for p in folder.iterdir():
            if linked(p): raise ValueError('Linked files cannot be published.')
            if p.is_dir(): collect(p)
            elif p.is_file() and p.name!='install-manifest.json':
                public_path(p.relative_to(dist).as_posix());paths.append(p)
    collect(dist);paths.sort()
    manifest={'version':VERSION,'files':{p.relative_to(dist).as_posix():hashlib.sha256(p.read_bytes()).hexdigest() for p in paths}}
    write_json(dist/'install-manifest.json',manifest);output.mkdir(parents=True)
    archive=output/ASSET_NAME
    with zipfile.ZipFile(archive,'x',zipfile.ZIP_DEFLATED,compresslevel=6) as z:
        for p in [*paths,dist/'install-manifest.json']: z.write(p,'CodexManager/'+p.relative_to(dist).as_posix())
    with zipfile.ZipFile(archive) as z:
        if z.testzip(): raise ValueError('압축파일 CRC 검증 실패')
    validate_archive(archive,VERSION)
    metadata={'version':VERSION,'asset':ASSET_NAME,'size':archive.stat().st_size,'sha256':hashlib.sha256(archive.read_bytes()).hexdigest()}
    write_json(output/'update.json',metadata)
    expanded_size=sum(p.stat().st_size for p in [*paths,dist/'install-manifest.json'])
    (output/'install.ini').write_text('[release]\nprotocol=1\n'+''.join(f'{key}={metadata[key]}\n' for key in ('version','size','sha256'))+f'expanded_size={expanded_size}\n',encoding='ascii')
    checksums=''.join(hashlib.sha256((output/name).read_bytes()).hexdigest()+'  '+name+'\n' for name in (ASSET_NAME,'update.json','install.ini'))
    (output/'SHA256SUMS.txt').write_text(checksums,encoding='utf-8')
    print(json.dumps(metadata));return archive


if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('--dist',type=Path,default=Path('dist/CodexManager'));parser.add_argument('--output',type=Path,default=Path('releases')/('v'+VERSION))
    args=parser.parse_args();build(args.dist,args.output)
