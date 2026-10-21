from pathlib import Path
import argparse,sys
sys.path.insert(0,str(Path(__file__).resolve().parents[2]))
from engine.papers import create,hash_index,intake
from engine.paths import external

def main():
 p=argparse.ArgumentParser(description='Make paper folders from files, a folder (read in place), or the inbox when nothing is given')
 p.add_argument('sources',nargs='*'); p.add_argument('--title'); p.add_argument('--id'); p.add_argument('--series',default=''); p.add_argument('--group',default='')
 p.add_argument('--priority',action='store_true'); p.add_argument('--limit',type=int); a=p.parse_args()
 sources=a.sources or [str(external('inbox'))]; made=[]; known=hash_index(); start=len(known)
 for source in sources:
  left=None if a.limit is None else a.limit-(len(known)-start)  # --limit counts new paper folders across all sources
  if left is not None and left<=0: break
  s=Path(source).expanduser()
  if s.is_dir(): made+=intake(s,series=a.series,group=a.group,priority=a.priority,known=known,limit=left)
  else: made.append(create(s,title=a.title,paper_id=a.id,series=a.series,group=a.group,priority=a.priority,known=known))
 for m in dict.fromkeys(made): print(m)
if __name__=='__main__': main()
