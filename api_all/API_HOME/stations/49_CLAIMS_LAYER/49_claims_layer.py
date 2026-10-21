from pathlib import Path
import sys
sys.path.insert(0,str(Path(__file__).resolve().parents[2]))
from engine.paper_station import run_ai
if __name__=='__main__':
 raise SystemExit(run_ai('49_CLAIMS_LAYER',[
 ('claims',(Path(__file__).resolve().parent/'PROMPT.md').read_text(encoding='utf-8-sig'))]))
