from __future__ import annotations
import argparse,hashlib,html,json,re,shutil,time
from concurrent.futures import ThreadPoolExecutor,as_completed
from pathlib import Path
from . import llm
from .focus import append,compose
from .output import dated_run_dir,sha256_file,write_bundle
from .papers import discover
from .paths import API_HOME

def arguments(label):
 p=argparse.ArgumentParser(prog=label); p.add_argument('items',nargs='*'); p.add_argument('--limit',type=int); p.add_argument('--workers',type=int,default=30); p.add_argument('--provider',default='deepseek'); p.add_argument('--model',default='deepseek-chat'); p.add_argument('--focus',default=''); p.add_argument('--redo',action='store_true'); p.add_argument('--template',default=''); p.add_argument('--vocab',action='append',default=[]); p.add_argument('--dry-run',action='store_true'); return p.parse_args()
AFTER_CALL=('provider','exact_model_string','ISO_8601_timestamp','prompt_completion_total_cost_object')
def output_template(label,name):
 """--template NAME|path, else the station's default "template" in station.json, else none."""
 station=API_HOME/'stations'/label/'station.json'
 name=name or (json.loads(station.read_text(encoding='utf-8')).get('template','') if station.is_file() else '')
 if not name: return ''
 path=Path(name).expanduser() if Path(name).suffix=='.md' else API_HOME/'templates'/'OUTPUT'/f'{name}.md'
 if not path.is_file(): raise FileNotFoundError(f'output template not found: {path}')
 return path.read_text(encoding='utf-8-sig')
CODE=re.compile(r'`([^`\s]+)`')
def vocabularies(label,names):
 """--vocab NAME|path (repeatable), else the station's "vocab" default (a name or a list), else none."""
 station=API_HOME/'stations'/label/'station.json'
 if not names and station.is_file():
  d=json.loads(station.read_text(encoding='utf-8')).get('vocab',[]); names=[d] if isinstance(d,str) else d
 texts=[]
 for n in names:
  path=Path(n).expanduser() if Path(n).suffix=='.md' else API_HOME/'templates'/'VOCAB'/f'{n}.md'
  if not path.is_file(): raise FileNotFoundError(f'vocabulary not found: {path}')
  texts.append(path.read_text(encoding='utf-8-sig'))
 return texts
def vocab_block(texts):
 # Goes first in the prompt: the same opening for every paper lets the provider's prompt cache bill it at the cache-hit rate.
 if not texts: return ''
 return ('REFERENCE VOCABULARY (read it; do not reproduce it). Classify everything you report using only the codes below, '
  'each written in backticks exactly as listed. Report only what is actually present in the document; never list a category '
  'that has nothing in it. If something fits no code, write `UNCLASSIFIED` and say why in the same row.\n\n'+'\n\n---\n\n'.join(texts)+'\n\nEND OF REFERENCE VOCABULARY\n\n')
def check_codes(text,vocab_texts,template):
 """Codes the reply used, and any backticked code that is in no vocabulary (closed vocabulary: those are flagged, never silently kept)."""
 allowed=set(CODE.findall('\n'.join(vocab_texts)+template))|{'UNCLASSIFIED'}
 used=CODE.findall(text); counts={}
 for c in used: counts[c]=counts.get(c,0)+1
 return {'codes_used':{c:n for c,n in sorted(counts.items()) if c in allowed},'codes_unknown':sorted({c for c in used if c not in allowed})}
def with_template(instruction,template,meta,src_hash):
 if not template: return instruction
 t=template.replace('{{paper_id}}',str(meta.get('id',''))).replace('{{source_sha256}}',src_hash)
 return (instruction+'\n\nReturn your answer by filling in the OUTPUT TEMPLATE below. Keep every heading, table and column exactly as given; '
  'add one row per item with no cap; replace every {{placeholder}} you can fill from the document. Leave '+', '.join('{{'+k+'}}' for k in AFTER_CALL)+
  ' exactly as they are: they are filled in after the call. Return only the filled template.\n\nOUTPUT TEMPLATE:\n'+t)
def after_call(text,result):
 values={'provider':result.provider,'exact_model_string':result.model,'ISO_8601_timestamp':result.started_at,
  'prompt_completion_total_cost_object':json.dumps({'prompt':result.prompt_tokens,'completion':result.completion_tokens,'total':result.prompt_tokens+result.completion_tokens})}
 for k,v in values.items(): text=text.replace('{{'+k+'}}',v)
 return text
def source(paper):
 meta=json.loads((paper/'paper.json').read_text(encoding='utf-8')); path=paper/'00_SOURCE'/meta['source_file']; return meta,path,path.read_text(encoding='utf-8',errors='replace')
def ai_bundle(paper:Path,label:str,prompts:list[tuple[str,str]],args):
 meta,path,text=source(paper); focus,focus_hash=compose(API_HOME/'stations'/label,paper,args.focus); src_hash=sha256_file(path)
 template=output_template(label,args.template); template_hash=hashlib.sha256(template.encode()).hexdigest() if template else ""
 vocab=vocabularies(label,args.vocab); opening=vocab_block(vocab)
 # Everything that changes the answer: editing a prompt, template or vocabulary (or dropping one) reruns the paper.
 checkpoint={"source_hash":src_hash,"prompt_version":"1","model":args.model,"provider":args.provider,"focus_hash":focus_hash,
  "prompts_hash":hashlib.sha256(json.dumps(prompts,ensure_ascii=False).encode()).hexdigest(),"template_hash":template_hash,
  "vocab_hash":hashlib.sha256(opening.encode()).hexdigest() if vocab else ""}
 if not args.redo:
  for prior in sorted((paper/"02_RUNS"/label).glob("*/"+label+".run.json"),reverse=True):
   saved=json.loads(prior.read_text(encoding="utf-8"))
   if all(saved.get(k)==v for k,v in checkpoint.items()) and not saved.get("errors"):
    print(f"SKIP completed {paper}: {prior.parent}"); return True
 def one(entry):
  name,instruction=entry; prompt=append(opening+with_template(instruction,template,meta,src_hash)+'\n\nSOURCE DOCUMENT:\n'+text,focus); result=llm.call([{'role':'user','content':prompt}],provider=args.provider,model=args.model)
  if template and not result.error: result.text=after_call(result.text,result)
  return name,result,prompt
 gathered={}; receipts=[]
 with ThreadPoolExecutor(max_workers=min(args.workers,len(prompts))) as pool:
  futures=[pool.submit(one,p) for p in prompts]
  for f in as_completed(futures):
   name,result,prompt=f.result(); gathered[name]=result.text if not result.error else {'error':result.error}; receipts.append(llm.receipt(result,source_hash=src_hash,prompt_version='1',focus_text=focus,focus_hash=focus_hash,part=name,**(check_codes(result.text,vocab,template) if vocab and not result.error else {})))
 out=dated_run_dir(paper,label); receipt={**checkpoint,'focus_text':focus,**({'codes_unknown':sorted({c for r in receipts for c in r.get('codes_unknown',[])})} if vocab else {}),'tokens':sum(r['tokens'] for r in receipts),'time_seconds':sum(r['elapsed_seconds'] for r in receipts),'errors':[r['error'] for r in receipts if r['error']],'calls':receipts}
 # Until every artifact is written the receipt says so; a crash midway leaves a run that is redone, never skipped.
 pending={**receipt,'errors':receipt['errors']+['incomplete: run did not finish writing its outputs']}
 body='<h1>'+html.escape(label)+'</h1>'+''.join(f'<section><h2>{html.escape(k)}</h2><pre>{html.escape(v if isinstance(v,str) else json.dumps(v,indent=2))}</pre></section>' for k,v in gathered.items()); write_bundle(out,label,gathered,'<!doctype html><meta charset="utf-8">'+body,pending,[{'part':k,'result':v if isinstance(v,str) else json.dumps(v)} for k,v in gathered.items()])
 if template:
  parts=[(k,v if isinstance(v,str) else '(call failed: '+v['error']+')') for k,v in gathered.items()]
  (out/f'{label}.md').write_text(parts[0][1] if len(parts)==1 else '\n\n'.join(f'# {k}\n\n{v}' for k,v in parts),encoding='utf-8')
 mirror(paper,meta,label,out)
 meta.setdefault('stations_run',[]).append({'station':label,'run':str(out.relative_to(paper))}); (paper/'paper.json').write_text(json.dumps(meta,indent=2),encoding='utf-8')
 (out/f'{label}.run.json').write_text(json.dumps(receipt,indent=2,ensure_ascii=False),encoding='utf-8'); print(out)
 return not receipt['errors']

def mirror(paper:Path,meta:dict,label:str,out:Path):
 """A paper in a series or group also gets a copy of this station's result in <collection>/_BY_STATION/<label>/,
 so every paper's output from one station sits together. The paper folder stays the master copy."""
 if not (meta.get('series') or meta.get('group')): return
 dest=paper.parent/'_BY_STATION'/label; dest.mkdir(parents=True,exist_ok=True)
 for f in out.glob(label+'.*'):
  if not f.name.endswith('.run.json'): shutil.copy2(f,dest/(paper.name+f.suffix))

def run_ai(label,prompts):
 args=arguments(label); papers=[x for x in discover(args.items,args.limit) if x.is_dir()]
 if args.dry_run:
  print(json.dumps({'station':label,'papers':len(papers),'calls':len(papers)*len(prompts),'template':bool(output_template(label,args.template)),'vocabularies':len(vocabularies(label,args.vocab)),'workers':args.workers})); return 0
 # Papers run side by side; the one process-wide limiter in engine.llm caps calls in flight at --workers.
 llm.configure(args.workers)
 def guarded(paper):
  try: return ai_bundle(paper,label,prompts,args)
  except Exception as exc: print(f'{paper}: {type(exc).__name__}: {exc}'); return False
 with ThreadPoolExecutor(max_workers=max(1,min(args.workers,len(papers)))) as pool: ok=list(pool.map(guarded,papers))
 failed=ok.count(False)
 if failed: print(f'{label}: {failed} of {len(papers)} paper(s) failed or have call errors; rerun to retry them')
 return 1 if failed else 0  # stations pass this to SystemExit, so ONE_MENU counts the station as failed
