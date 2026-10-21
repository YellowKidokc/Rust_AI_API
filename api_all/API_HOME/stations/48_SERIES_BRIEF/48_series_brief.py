"""Series brief: each paper read whole (in parallel), then one series analysis over those briefs."""
from pathlib import Path
import argparse,hashlib,json,sys
from datetime import datetime,timezone
from concurrent.futures import ThreadPoolExecutor
sys.path.insert(0,str(Path(__file__).resolve().parents[2]))
from engine import llm
from engine.focus import append,compose
from engine.papers import all_papers,run_order
from engine.paths import external

LABEL='48_SERIES_BRIEF'; HERE=Path(__file__).resolve().parent

def prompt_file(name):
 # Lines starting with "# " are notes for David; "## " headings belong to the prompt.
 return '\n'.join(l for l in (HERE/name).read_text(encoding='utf-8-sig').splitlines() if not l.startswith('# ')).strip()
def fill(text,**values):
 for k,v in values.items(): text=text.replace('{{'+k+'}}',str(v))
 return text
def parse_json(text):
 try: return json.loads(text[text.index('{'):text.rindex('}')+1])
 except ValueError: return {'raw':text}
def collections(items):
 if items: return [Path(i).expanduser().resolve() for i in items]
 root=external('papers_root')/'SERIES'
 return sorted(p for p in root.iterdir() if p.is_dir()) if root.is_dir() else []
def members(folder):
 return sorted((p for p in all_papers(folder) if p.parent==folder),key=run_order)
def source_text(paper):
 meta=json.loads((paper/'paper.json').read_text(encoding='utf-8'))
 return meta,(paper/'00_SOURCE'/meta['source_file']).read_text(encoding='utf-8',errors='replace')

def brief(folder,args):
 papers=members(folder)
 if not papers: print(f'{folder}: no papers'); return True
 kind='series' if folder.parent.name=='SERIES' else 'group'
 paper_prompt,series_prompt=prompt_file('PAPER_PROMPT.md'),prompt_file('SERIES_PROMPT.md')
 focus,focus_hash=compose(HERE,None,args.focus)
 metas=[source_text(p) for p in papers]
 key=hashlib.sha256(json.dumps([[m['source_hash'] for m,_ in metas],paper_prompt,series_prompt,args.model,focus_hash]).encode()).hexdigest()
 if not args.redo:
  for prior in sorted((folder/'_SERIES').glob(f'*/{LABEL}.run.json'),reverse=True):
   saved=json.loads(prior.read_text(encoding='utf-8'))
   if saved.get('checkpoint')==key and not saved.get('errors'): print(f'SKIP unchanged {folder.name}: {prior.parent}'); return True
 print(json.dumps({'collection':str(folder),'kind':kind,'papers':len(papers),'calls':len(papers)+1}))
 if args.dry_run: return True
 titles=[m.get('title') or p.name for p,(m,_) in zip(papers,metas)]
 def one(i):
  m,text=metas[i]
  p=fill(paper_prompt,collection=folder.name,position=f'paper {i+1} of {len(papers)}',
         previous=titles[i-1] if i else 'none (first)',next=titles[i+1] if i+1<len(papers) else 'none (last)')
  return i,llm.call([{'role':'user','content':append(p,focus)+'\nSOURCE DOCUMENT:\n'+text}],provider=args.provider,model=args.model)
 with ThreadPoolExecutor(max_workers=max(1,min(args.workers,len(papers)))) as pool: stage1=sorted(pool.map(one,range(len(papers))))
 briefs=[]; receipts=[]
 for i,r in stage1:
  briefs.append({'paper':papers[i].name,'title':titles[i],**(parse_json(r.text) if not r.error else {'error':r.error})})
  receipts.append(llm.receipt(r,part=papers[i].name,source_hash=metas[i][0]['source_hash']))
 block='\n\n'.join(f"### {i+1}. {b['title']}\n{json.dumps(b,ensure_ascii=False,indent=1)}" for i,b in enumerate(briefs))
 final=llm.call([{'role':'user','content':append(fill(series_prompt,collection=folder.name,kind=kind,count=len(papers),briefs=block),focus)}],provider=args.provider,model=args.model)
 receipts.append(llm.receipt(final,part='series'))
 out=folder/'_SERIES'/datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ'); out.mkdir(parents=True,exist_ok=False)
 errors=[r['error'] for r in receipts if r['error']]
 (out/f'{LABEL}.md').write_text(f'# {folder.name}: {kind} brief\n\n'+(final.text or f'(series call failed: {final.error})')+'\n',encoding='utf-8')
 (out/f'{LABEL}.json').write_text(json.dumps({'collection':folder.name,'kind':kind,'papers':briefs,'brief_markdown':final.text},indent=2,ensure_ascii=False),encoding='utf-8')
 (out/f'{LABEL}.run.json').write_text(json.dumps({'checkpoint':key,'model':args.model,'provider':args.provider,'focus_hash':focus_hash,
   'tokens':sum(r['tokens'] for r in receipts),'errors':errors,'calls':receipts},indent=2,ensure_ascii=False),encoding='utf-8')
 print(out)
 return not errors

def main():
 p=argparse.ArgumentParser(prog=LABEL); p.add_argument('items',nargs='*'); p.add_argument('--workers',type=int,default=30)
 p.add_argument('--provider',default='deepseek'); p.add_argument('--model',default='deepseek-chat'); p.add_argument('--focus',default='')
 p.add_argument('--redo',action='store_true'); p.add_argument('--dry-run',action='store_true'); args=p.parse_args()
 llm.configure(args.workers); failed=0
 for folder in collections(args.items):
  try: ok=brief(folder,args)
  except Exception as exc: print(f'{folder}: {type(exc).__name__}: {exc}'); ok=False
  failed+=not ok
 return 1 if failed else 0  # ONE_MENU counts the station as failed
if __name__=='__main__': raise SystemExit(main())
