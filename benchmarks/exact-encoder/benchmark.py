#!/usr/bin/env python3
"""Portable controls-only wrapper around the unchanged frozen mechanical worker."""
import argparse
import hashlib
import importlib.util
import json
import os
import re
from pathlib import Path
import shutil
import subprocess
import sys
import analysis
ROOT = Path(__file__).resolve().parent
WORKER_SHA = '866235a79a0b6e0bde6873efcb17613219a4736c5e74913c26b1611bd880771d'
CONTROL_REVISIONS = {'current':'d4664689177ad50c5b94906e882e4a36fd3aca8f','wide':'c1d7fc8e90392074c15840a864bc88e119d607e8','cyclic':'822681395d84a0523519b584d3a64ee8f7b370aa'}

def sha(p): return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def read(p): return json.loads(Path(p).read_text())
def write(p,v):
    p=Path(p); t=p.with_suffix(p.suffix+'.tmp');t.write_text(json.dumps(v,sort_keys=True,indent=2)+'\n');os.replace(t,p)
def require(x,s):
    if not x: raise RuntimeError(s)
def call(args,log):
    with Path(log).open('a') as out: subprocess.run(list(map(str,args)),stdout=out,stderr=subprocess.STDOUT,check=True)
def worker(run):
    s=importlib.util.spec_from_file_location('frozen_worker',run/'frozen/hybrid_candidates.py');w=importlib.util.module_from_spec(s);s.loader.exec_module(w);return w

def freeze(a):
    require(re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9_.-]*',a.label),'Unsafe label')
    require(sha(ROOT/'vendor/worker.py')==WORKER_SHA,'Pinned worker changed')
    clean=subprocess.check_output(['git','-C',str(a.repository),'status','--porcelain','--untracked-files=all'],text=True)
    require(not clean,'Freeze requires a clean committed candidate repository')
    references=read(ROOT/'references/manifest.json')
    supplied={'current':a.current,'wide':a.wide,'cyclic':a.cyclic}
    historical={'current':'6c6a5ef616f69bb428933f1a6d42a86a2434d09477ec6339b12cbe8cdcf938dd',
                **{r:v['historical_local_binary_sha256'] for r,v in references['references'].items()}}
    if a.control_provenance:
        provenance=read(a.control_provenance)
        require(not provenance.get('check_only') and provenance['base_revision']==references['base_revision']
                and provenance['reference_manifest_sha256']==sha(ROOT/'references/manifest.json'),'Unverified control build receipt')
        for role,binary in supplied.items():
            record=provenance['controls'][role]
            require(record['binary_sha256']==sha(binary) and record['base_revision']==references['base_revision'],'Control binary/source mismatch')
            if role!='current':require(record['patch_sha256']==references['references'][role]['patch_sha256']
                and record['feature']==references['references'][role]['feature'],'Control patch/feature mismatch')
        control_basis='source/patch/compiler receipt bound to each supplied binary SHA256'
    else:
        require(all(sha(b)==historical[r] for r,b in supplied.items()),'Unknown control binary: supply the verified build_controls.py receipt')
        control_basis='known historical frozen binary SHA256, bound to the source reference manifest'
    expected=read(ROOT/'corpus.json');manifest=read(a.corpus)
    require(manifest==expected,'Use the exact generated corpus manifest')
    out=a.output.resolve();out.mkdir(parents=True,exist_ok=False)
    # Preserve the wrapper/analysis used for this experiment; resume uses these.
    for name in ('benchmark.py','analysis.py','assess.py','native_smoke.py'):
        shutil.copy2(ROOT/name,out/name)
    shutil.copy2(ROOT/'plan.json',out/'plan.json')
    shutil.copy2(ROOT/'references/manifest.json',out/'reference-sources.json')
    candidate_hash,native_hash=sha(a.candidate),sha(a.native_rar)
    first_cases=None
    roles={'current':a.current,'wide':a.wide,'cyclic':a.cyclic}
    meta={'label':a.label,'controls':{},'corpus':str(a.corpus.resolve()),'settings_sha256':sha(ROOT/'plan.json'),
          'scope':'Original fixed local protocol; operator supplies controls built from reference sources. No build equivalence across platforms is implied.',
          'control_source_basis':control_basis,'reference_sources_sha256':sha(out/'reference-sources.json'),
          'compiler':subprocess.check_output(['rustc','--version','--verbose'],text=True),'worker_sha256':WORKER_SHA}
    meta['compiler_scope']='Freezer environment; supplied prebuilt binary compiler identity requires the optional control/build receipt.'
    meta['helpers_sha256']={name:sha(out/name) for name in ('benchmark.py','analysis.py','assess.py','native_smoke.py')}
    if a.control_provenance:
        shutil.copy2(a.control_provenance,out/'control-build.json');meta['control_build_sha256']=sha(out/'control-build.json')
    for name,binary in roles.items():
        run=out/('run-'+name)
        call([sys.executable,ROOT/'vendor/worker.py','init','--output',run,'--experiment','controls','--profile','desktop',
              '--repository',a.repository,'--rars',a.candidate,'--baseline',binary,'--native-rar',a.native_rar,
              '--corpus',a.corpus,'--plan',ROOT/'plan.json','--seed','20261017'],out/'freeze.log')
        m=read(run/'manifest.json')
        require(m['confirmation_repetitions']==15 and len(m['cases'])==7,'Unexpected protocol')
        require(m['binaries']['candidate']['sha256']==candidate_hash and m['binaries']['native']['sha256']==native_hash
                and m['binaries']['baseline']['sha256']==sha(binary),'Binary changed during freeze')
        if first_cases is not None:require(m['cases']==first_cases,'Corpus/metadata changed between control freezes')
        first_cases=m['cases']
        meta['controls'][name]={'run':run.name,'source_revision':CONTROL_REVISIONS[name],'binary_sha256':sha(binary),'manifest_sha256':sha(run/'manifest.json')}
        if 'source' in meta: require(meta['source']==m['source'],'Source changed during freeze')
        meta['source']=m['source'];meta['candidate_sha256']=m['binaries']['candidate']['sha256']
    current=read(out/'run-current/manifest.json');case=next(c for c in current['cases'] if c['id']=='source-tree-2m')
    source_name=case['archive_members'][0];source=out/'run-current'/case['input_path']/source_name
    smoke=out/'smoke-corpus';smoke.mkdir();shutil.copy2(source,smoke/source_name)
    write(smoke/'manifest.json',{'files':[{'path':source_name,'size':source.stat().st_size,'sha256':sha(source)}]})
    meta['smoke_manifest_sha256']=sha(smoke/'manifest.json');meta['smoke_source_sha256']=sha(source)
    write(out/'experiment.json',meta);(out/'experiment.sha256').write_text(sha(out/'experiment.json')+'\n')
    print(out)

def load(out):
    require(sha(out/'experiment.json')==(out/'experiment.sha256').read_text().strip(),'Experiment metadata changed')
    meta=read(out/'experiment.json')
    require(all(sha(out/n)==h for n,h in meta['helpers_sha256'].items()),'Frozen orchestration/analysis changed')
    require(sha(Path(__file__))==meta['helpers_sha256']['benchmark.py'] and sha(Path(analysis.__file__))==meta['helpers_sha256']['analysis.py'],'Use the frozen wrapper and analysis')
    require(sha(out/'plan.json')==meta['settings_sha256'],'Frozen settings changed')
    require(sha(out/'reference-sources.json')==meta['reference_sources_sha256'],'Control source identity changed')
    if 'control_build_sha256' in meta:require(sha(out/'control-build.json')==meta['control_build_sha256'],'Control build receipt changed')
    for control,record in meta['controls'].items():
        run=out/record['run'];require(sha(run/'manifest.json')==record['manifest_sha256'],'Frozen control manifest changed')
        worker(run).load(run)
    require(sha(out/'smoke-corpus/manifest.json')==meta['smoke_manifest_sha256'] and sha(out/'smoke-corpus/source-tree-2m.rs')==meta['smoke_source_sha256'],'Frozen smoke corpus changed')
    return meta
def screen(out):
    meta=load(out);run=out/'run-current';w=worker(run);m=w.load(run)
    expected={w.task_id(t) for t in w.tasks(m,'screen')}
    for _ in range(7):
        events=w.latest(w.journal(run))
        require(all(e['event']=='complete' for e in events.values()),'Failed/unfinished task retained; use its receipts')
        rows=w.verified_results(run,w.journal(run),'screen')
        covered={w.task_id(r['task']) for r in rows}
        if covered==expected:break
        call([sys.executable,run/'frozen/hybrid_candidates.py','run',run,'--max-tasks','1'],out/'screen.log')
        after=w.verified_results(run,w.journal(run),'screen')
        require(len(after)>len(rows),'No screen progress: budget exhausted or task refused; preserve incomplete coverage')
    rows=w.verified_results(run,w.journal(run),'screen')
    require({w.task_id(r['task']) for r in rows}==expected,'Seven-case screen incomplete')
    verification=analysis.verification_counts(w,run,m,rows,'screen')
    require(all(r['roles']['candidate']['archive_sha256']==r['roles']['baseline']['archive_sha256'] for r in rows),'Archive identity failed')
    write(out/'screen.json',{'all_seven_archive_hashes_match_current':True,'verification':verification,'source':m['source']})

def qualify(out):
    meta=load(out);require(read(out/'screen.json')['all_seven_archive_hashes_match_current'],'Screen first')
    loaded={}
    for control in ('current','wide','cyclic'):
        run=out/('run-'+control)
        call([sys.executable,run/'frozen/hybrid_candidates.py','run',run],out/('qualification-'+control+'.log'))
        loaded[control]=analysis.load_run(run,control)
    workloads={};verifications={}
    for control,(w,m,all_rows,measured) in loaded.items():
        verifications[control]=analysis.verification_counts(w,out/('run-'+control),m,all_rows,control)
    for case in loaded['current'][1]['cases']:
        name=case['id'];chosen={c:sorted((r for r in v[3] if r['task']['case']==name),key=lambda r:r['task']['repetition']) for c,v in loaded.items()}
        hashes={r['roles']['candidate']['archive_sha256'] for v in loaded.values() for r in v[2] if r['task']['case']==name}
        require(len(hashes)==1,'Candidate bytes differ across runs')
        require(all(r['roles']['candidate']['archive_sha256']==r['roles']['baseline']['archive_sha256'] for r in loaded['current'][2] if r['task']['case']==name),'Current archive identity changed')
        value={'archive_sha256':next(iter(hashes)),'measured_pairs_per_comparison':15,
               'candidate':analysis.role_metrics(chosen['current'],'candidate'),'current':analysis.role_metrics(chosen['current'],'baseline'),
               'prior_wide':analysis.role_metrics(chosen['wide'],'baseline'),'cyclic_cache':analysis.role_metrics(chosen['cyclic'],'baseline')}
        for control,key,label in [('current','candidate_current','current'),('wide','candidate_prior','prior'),('cyclic','candidate_cyclic','cyclic')]:
            value[key]={metric:analysis.bootstrap([r['roles']['candidate']['encode'][metric]/r['roles']['baseline']['encode'][metric] for r in chosen[control]],meta['label']+'/'+name+'/'+label+'/'+metric) for metric in ('wall_seconds','cpu_seconds','peak_rss_bytes')}
        workloads[name]=value
    result={'label':meta['label'],'source':meta['source'],'candidate_sha256':meta['candidate_sha256'],'settings':analysis.settings(loaded['current'][1]),
            'exact_current_archive_identity_all_workloads':True,'workloads':workloads,'scope':'Three fresh serial controls, 15 original standalone measured pairs per case. No default promotion.'}
    for c,k in [('current','current_run'),('wide','prior_run'),('cyclic','cyclic_run')]:result[k]={'path':str(out/('run-'+c)),'verification':verifications[c],'manifest_sha256':meta['controls'][c]['manifest_sha256']}
    write(out/'qualification.json',result)
    print(out/'qualification.json')

def main():
    p=argparse.ArgumentParser(description=__doc__);s=p.add_subparsers(dest='command',required=True)
    f=s.add_parser('freeze')
    for name in ('repository','candidate','current','wide','cyclic','native-rar','corpus','output'):f.add_argument('--'+name,type=Path,required=True)
    f.add_argument('--control-provenance',type=Path);f.add_argument('--label',required=True)
    for name in ('screen','qualify','status','smoke','export'):
        c=s.add_parser(name);c.add_argument('--output',type=Path,required=True)
        if name=='export':c.add_argument('--destination',type=Path,required=True)
    a=p.parse_args()
    if a.command=='freeze':freeze(a);return
    out=a.output.resolve()
    if a.command=='screen':screen(out)
    elif a.command=='qualify':qualify(out)
    elif a.command=='export':
        meta=load(out);q=read(out/'qualification.json')
        summary={k:q[k] for k in ('label','source','candidate_sha256','settings','exact_current_archive_identity_all_workloads','workloads','scope')}
        summary['controls']={k:{x:v[x] for x in ('source_revision','binary_sha256','manifest_sha256')} for k,v in meta['controls'].items()}
        summary['compiler']=meta['compiler'];summary['compiler_scope']=meta['compiler_scope'];summary['worker_sha256']=meta['worker_sha256']
        summary['verification']={k:q[k]['verification'] for k in ('current_run','prior_run','cyclic_run')}
        write(a.destination,summary)
    elif a.command=='status':
        for c in ('current','wide','cyclic'):call([sys.executable,out/('run-'+c)/'frozen/hybrid_candidates.py','status',out/('run-'+c)],out/('status-'+c+'.json'))
    else:
        meta=load(out);m=read(out/'run-current/manifest.json');base=out/'run-current'
        # Native smoke uses the frozen full-size source fixture.
        call([sys.executable,out/'native_smoke.py',meta['label'],base/m['binaries']['candidate']['path'],'--source-revision',meta['source']['revision'],
              '--native-rar',base/m['binaries']['native']['path'],'--corpus',out/'smoke-corpus','--output',out/'native-smoke'],out/'native-smoke.log')
if __name__=='__main__':main()
