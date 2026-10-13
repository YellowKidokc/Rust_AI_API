from pathlib import Path
import argparse,sys
sys.path.insert(0,str(Path(__file__).resolve().parents[2]))
from engine.papers import discover
from engine.site import build

def main():
 p=argparse.ArgumentParser(description='Build the front side: dashboard, index.tsv, validation and one stacked page per paper')
 p.add_argument('items',nargs='*'); p.add_argument('--limit',type=int)
 p.add_argument('--order',default='',help='stacking order of layers by station number, e.g. 49,40,41 (others follow)'); a=p.parse_args()
 papers=[x for x in discover(a.items,a.limit) if x.is_dir()] if a.items else None
 print(build(papers,[x.strip().zfill(2) for x in a.order.split(',') if x.strip()]))
if __name__=='__main__': main()
