#!/usr/bin/env python3
"""Build public d466 and two source-only frozen reference patches in new checkouts."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
ROOT=Path(__file__).resolve().parent

def sha(p): return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def execute(argv,cwd=None): subprocess.run(list(map(str,argv)),cwd=cwd,check=True)
def prepare(repository,output,check_only=False):
    spec=json.loads((ROOT/'references/manifest.json').read_text());output.mkdir(parents=True,exist_ok=False)
    result={'base_revision':spec['base_revision'],'reference_manifest_sha256':sha(ROOT/'references/manifest.json'),'controls':{},'check_only':check_only}
    if not check_only:
        result['compiler']=subprocess.check_output(['rustc','--version','--verbose'],text=True)
        result['cargo']=subprocess.check_output(['cargo','--version'],text=True)
    for role in ('current','wide','cyclic'):
        tree=output/('source-'+role)
        execute(['git','clone','--shared','--no-checkout',repository,tree]);execute(['git','checkout','--detach',spec['base_revision']],tree)
        reference=spec['references'].get(role);record={'base_revision':spec['base_revision']}
        if reference:
            patch=ROOT/'references'/reference['patch'];assert sha(patch)==reference['patch_sha256']
            execute(['git','apply','--check',patch],tree);execute(['git','apply',patch],tree)
            for changed in reference['changed_files']:
                assert sha(tree/changed['path'])==changed['sha256'],changed['path']
                actual=subprocess.check_output(['git','hash-object',changed['path']],cwd=tree,text=True).strip()
                assert actual==changed['git_blob'],changed['path']
            record.update({'patch_sha256':sha(patch),'original_revision':reference['original_revision'],'feature':reference['feature']})
        command=['cargo','build','--release','--locked','-p','rars-cli']
        if reference:command+=['--features',reference['feature']]
        record['command']=command
        if not check_only:
            execute(command,tree);binary=output/role;shutil.copy2(tree/'target/release/rars',binary)
            record.update({'binary':str(binary),'binary_sha256':sha(binary)})
        result['controls'][role]=record
    (output/'controls.json').write_text(json.dumps(result,sort_keys=True,indent=2)+'\n');return result
if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--repository',type=Path,required=True);p.add_argument('--output',type=Path,required=True);p.add_argument('--check-only',action='store_true')
    a=p.parse_args();prepare(a.repository.resolve(),a.output.resolve(),a.check_only)
