import json,os,sys,tempfile,threading,time,unittest
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
from pathlib import Path
from unittest.mock import patch
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from engine import paper_station
from engine.papers import create

class _Slow(BaseHTTPRequestHandler):
 lock=threading.Lock(); now=0; peak=0
 def do_POST(self):
  self.rfile.read(int(self.headers['Content-Length']))
  with _Slow.lock: _Slow.now+=1; _Slow.peak=max(_Slow.peak,_Slow.now)
  time.sleep(0.3)
  with _Slow.lock: _Slow.now-=1
  out=json.dumps({'choices':[{'message':{'content':'ok'}}],'usage':{'prompt_tokens':1,'completion_tokens':1}}).encode()
  self.send_response(200); self.send_header('Content-Type','application/json'); self.send_header('Content-Length',str(len(out))); self.end_headers(); self.wfile.write(out)
 def log_message(self,*_): pass

class ParallelPapersTests(unittest.TestCase):
 def test_papers_run_side_by_side_under_one_ceiling(self):
  server=ThreadingHTTPServer(('127.0.0.1',0),_Slow); threading.Thread(target=server.serve_forever,daemon=True).start()
  try:
   with tempfile.TemporaryDirectory() as td:
    root=Path(td); (root/'papers').mkdir(); config=root/'paths.json'
    config.write_text(json.dumps({'papers_root':str(root/'papers'),'catalog_db':str(root/'catalog.sqlite')}))
    env={'ONE_MENU_PATHS_FILE':str(config),'FORGE_URL':f'http://127.0.0.1:{server.server_port}','FORGE_ROUTE':'test'}
    with patch.dict(os.environ,env):
     papers=[]
     for i in range(6):
      src=root/f'p{i}.md'; src.write_text(f'Paper {i} text.'); papers.append(str(create(src)))
     _Slow.peak=0
     with patch.object(sys,'argv',['x',*papers,'--workers','8']): paper_station.run_ai('99_TEST',[('a','A'),('b','B')])
    self.assertGreater(_Slow.peak,2)   # more than one paper's prompts at once
    self.assertLessEqual(_Slow.peak,8) # never above --workers
    for p in papers: self.assertTrue(list(Path(p,'02_RUNS','99_TEST').glob('*/99_TEST.run.json')))
  finally: server.shutdown(); server.server_close()

if __name__=='__main__': unittest.main()
