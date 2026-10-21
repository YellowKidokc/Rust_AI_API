import json,os,subprocess,sys,tempfile,threading,unittest
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
from pathlib import Path
from unittest.mock import patch
HOME=Path(__file__).resolve().parents[1]; sys.path.insert(0,str(HOME))
from engine import paper_station
from engine.papers import discover,intake

class _Gateway(BaseHTTPRequestHandler):
 prompts=[]
 def do_POST(self):
  body=json.loads(self.rfile.read(int(self.headers['Content-Length']))); _Gateway.prompts.append(body['messages'][-1]['content'])
  out=json.dumps({'choices':[{'message':{'content':'{"title":"T","brief":"B"}'}}],'usage':{'prompt_tokens':1,'completion_tokens':1}}).encode()
  self.send_response(200); self.send_header('Content-Type','application/json'); self.send_header('Content-Length',str(len(out))); self.end_headers(); self.wfile.write(out)
 def log_message(self,*_): pass

class CollectionTests(unittest.TestCase):
 def setUp(self):
  self.td=tempfile.TemporaryDirectory(); root=Path(self.td.name); self.root=root
  inbox=root/'INBOX'; (root/'papers').mkdir()
  for rel,text in {'loose.md':'Loose.','00_PRIORITY/urgent.md':'Urgent.','SERIES/God Is/02 Second.md':'Second.',
                   'SERIES/God Is/01 First.md':'First.','GROUPS/One Pages/page.md':'A page.'}.items():
   f=inbox/rel; f.parent.mkdir(parents=True,exist_ok=True); f.write_text(text)
  self.inbox=inbox; config=root/'paths.json'
  config.write_text(json.dumps({'papers_root':str(root/'papers'),'catalog_db':'','inbox':str(inbox)}))
  self.env={'ONE_MENU_PATHS_FILE':str(config)}
 def tearDown(self): self.td.cleanup()

 def test_inbox_sets_priority_series_group_and_never_duplicates(self):
  with patch.dict(os.environ,self.env):
   made=intake(self.inbox); again=intake(self.inbox); order=discover([])
  self.assertEqual(5,len(made)); self.assertEqual(sorted(made),sorted(again))
  self.assertTrue((self.inbox/'loose.md').is_file())                      # originals stay put
  papers=self.root/'papers'
  self.assertEqual(['01','02'],sorted(p.name[:2] for p in (papers/'SERIES'/'God Is').iterdir()))
  self.assertTrue(sorted((papers/'SERIES'/'God Is').iterdir())[0].name.endswith('_first'))   # order not repeated in the title
  self.assertEqual(1,len(list((papers/'GROUPS'/'One Pages').iterdir())))
  names=[json.loads((p/'paper.json').read_text())['source_file'] for p in order]
  self.assertEqual('urgent.md',names[0])                                  # priority first
  self.assertLess(names.index('01 First.md'),names.index('02 Second.md')) # series in its order

 def test_station_copies_and_series_brief(self):
  server=ThreadingHTTPServer(('127.0.0.1',0),_Gateway); threading.Thread(target=server.serve_forever,daemon=True).start()
  env={**self.env,'FORGE_URL':f'http://127.0.0.1:{server.server_port}','FORGE_ROUTE':'test'}
  try:
   with patch.dict(os.environ,env):
    intake(self.inbox); series=self.root/'papers'/'SERIES'/'God Is'
    with patch.object(sys,'argv',['x',str(series)]): paper_station.run_ai('99_TEST',[('a','A')])
   copies=sorted(p.name for p in (series/'_BY_STATION'/'99_TEST').iterdir())
   self.assertEqual(2,len([c for c in copies if c.endswith('.json')]))
   self.assertTrue(copies[0].startswith('01_'))
   script=HOME/'stations'/'48_SERIES_BRIEF'/'48_series_brief.py'
   _Gateway.prompts.clear()
   first=subprocess.run([sys.executable,str(script),str(series)],env={**os.environ,**env},capture_output=True,text=True)
   self.assertEqual(0,first.returncode,first.stderr)
   self.assertEqual(3,len(_Gateway.prompts))                              # 2 whole papers + 1 series call
   self.assertIn('First.',_Gateway.prompts[0]+_Gateway.prompts[1])
   self.assertIn('paper 1 of 2',' '.join(_Gateway.prompts))
   brief=next((series/'_SERIES').glob('*/48_SERIES_BRIEF.md')).read_text()
   self.assertIn('God Is: series brief',brief)
   again=subprocess.run([sys.executable,str(script),str(series)],env={**os.environ,**env},capture_output=True,text=True)
   self.assertIn('SKIP unchanged',again.stdout); self.assertEqual(3,len(_Gateway.prompts))
  finally: server.shutdown(); server.server_close()

if __name__=='__main__': unittest.main()
