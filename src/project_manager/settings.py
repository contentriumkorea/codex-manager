"""Read damaged preferences safely and retain their original bytes on repair."""
import json,shutil,uuid
from .bundles import write_json


def read_settings(path):
    try:
        data=json.loads(path.read_text(encoding='utf-8'))
        return data if isinstance(data,dict) else {}
    except (ValueError,OSError): return {}


def valid_settings(value):
    return (isinstance(value,dict)
        and ('home' not in value or isinstance(value['home'],str))
        and ('backups' not in value or isinstance(value['backups'],list) and all(isinstance(p,str) and p for p in value['backups']))
        and ('startup_update_check' not in value or isinstance(value['startup_update_check'],bool)))


def save_settings(path,values):
    old=read_settings(path)
    if path.exists():
        try: raw=json.loads(path.read_text(encoding='utf-8'))
        except ValueError: raw=None
        if not valid_settings(raw): shutil.copyfile(path,path.with_name('settings.invalid-'+uuid.uuid4().hex+'.json'))
    write_json(path,{**old,**values})
