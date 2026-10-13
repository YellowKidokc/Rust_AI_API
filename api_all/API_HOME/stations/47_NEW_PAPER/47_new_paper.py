from pathlib import Path
import argparse,sys
sys.path.insert(0,str(Path(__file__).resolve().parents[2]))
from engine.papers import create,intake
from engine.paths import external

def main():
 p=argparse.ArgumentParser(description='Make paper folders from files, a folder (read in place), or the inbox when nothing is given')
 p.add_argument('sources',nargs='*'); p.add_argument('--title'); p.add_argument('--id'); p.add_argument('--series',default=''); p.add_argument('--group',default='')
 p.add_argument('--priority',action='store_true'); p.add_argument('--limit',type=int); a=p.parse_args()
 sources=a.sources or [str(external('inbox'))]; made=[]
 for source in sources:
  s=Path(source).expanduser()
  if s.is_dir(): made+=intake(s,series=a.series,group=a.group,priority=a.priority)
  else: made.append(create(s,title=a.title,paper_id=a.id,series=a.series,group=a.group,priority=a.priority))
 for m in dict.fromkeys(made): print(m)
if __name__=='__main__': main()
