"""Regression tests for the CodeRabbit review on PR #3."""
import json,os,subprocess,sys,tempfile,threading,unittest
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
from pathlib import Path
from unittest.mock import patch
HOME=Path(__file__).resolve().parents[1]; sys.path.insert(0,str(HOME))
from engine import llm,paper_station,site
from engine.papers import intake

class _Gateway(BaseHTTPRequestHandler):
 reply={'choices':[{'message':{'content':'ok'}}],'usage':{'prompt_tokens':1,'completion_tokens':1}}; calls=0
 def do_POST(self):
  self.rfile.read(int(self.headers['Content-Length'])); _Gateway.calls+=1
  out=json.dumps(_Gateway.reply).encode()
  self.send_response(200); self.send_header('Content-Type','application/json'); self.send_header('Content-Length',str(len(out))); self.end_headers(); self.wfile.write(out)
 def log_message(self,*_): pass

class Base(unittest.TestCase):
 def setUp(self):
  _Gateway.reply={'choices':[{'message':{'content':'ok'}}],'usage':{'prompt_tokens':1,'completion_tokens':1}}; _Gateway.calls=0
  self.server=ThreadingHTTPServer(('127.0.0.1',0),_Gateway); threading.Thread(target=self.server.serve_forever,daemon=True).start()
  self.td=tempfile.TemporaryDirectory(); self.root=root=Path(self.td.name); (root/'papers').mkdir()
  (root/'paths.json').write_text(json.dumps({'papers_root':str(root/'papers'),'catalog_db':'','inbox':str(root/'INBOX')}))
  self.env={'ONE_MENU_PATHS_FILE':str(root/'paths.json'),'FORGE_URL':f'http://127.0.0.1:{self.server.server_port}','FORGE_ROUTE':'test'}
  self.patch=patch.dict(os.environ,self.env); self.patch.start()
 def tearDown(self): self.patch.stop(); self.server.shutdown(); self.server.server_close(); self.td.cleanup()
 def paper(self,name='p',text='Grace restores order.'):
  inbox=self.root/'INBOX'; inbox.mkdir(exist_ok=True); (inbox/f'{name}.md').write_text(text); return intake(inbox)[-1]
 def run_station(self,paper,prompts,*extra):
  with patch.object(sys,'argv',['x',str(paper),*extra]): return paper_station.run_ai('99_TEST',prompts)

class LlmTests(Base):
 def test_malformed_200_is_an_error_result_not_an_exception(self):
  for bad in ({'error':{'message':'route failed'}},{'choices':[{'message':{'content':None}}]}):
   _Gateway.reply=bad; r=llm.call([{'role':'user','content':'hi'}])
   self.assertTrue(r.error.startswith('MalformedResponse')); self.assertEqual('',r.text)
 def test_records_the_model_that_served_the_call(self):
  _Gateway.reply={**_Gateway.reply,'model':'claude-sonnet-5-5'}
  self.assertEqual('claude-sonnet-5-5',llm.call([{'role':'user','content':'hi'}]).model)

class PaperStationTests(Base):
 def test_exit_status_reflects_failed_calls(self):
  p=self.paper(); self.assertEqual(0,self.run_station(p,[('a','A')]))
  _Gateway.reply={'error':'down'}; self.assertEqual(1,self.run_station(p,[('a','A')],'--redo'))
 def test_editing_a_prompt_reruns_the_paper(self):
  p=self.paper(); self.run_station(p,[('a','A')]); self.run_station(p,[('a','A')]); self.assertEqual(1,_Gateway.calls)
  self.run_station(p,[('a','A, now asking something else')]); self.assertEqual(2,_Gateway.calls)
 def test_a_run_that_dies_midway_is_redone_not_skipped(self):
  p=self.paper()
  with patch.object(paper_station,'mirror',side_effect=OSError('disk full')): self.assertEqual(1,self.run_station(p,[('a','A')]))
  receipt=json.loads(next((p/'02_RUNS'/'99_TEST').glob('*/99_TEST.run.json')).read_text())
  self.assertTrue(any('incomplete' in e for e in receipt['errors']))
  self.assertEqual(0,self.run_station(p,[('a','A')])); self.assertEqual(2,_Gateway.calls)

class MenuAndIntakeTests(Base):
 def test_dry_run_never_executes_stations_without_a_dry_run_mode(self):
  inbox=self.root/'INBOX'; inbox.mkdir(); (inbox/'a.md').write_text('x')
  out=subprocess.run([sys.executable,str(HOME/'engine'/'menu.py'),'47','92','--dry-run','--yes'],capture_output=True,text=True,env={**os.environ},stdin=subprocess.DEVNULL)
  self.assertEqual(0,out.returncode,out.stderr); self.assertIn('dry run: not executed',out.stdout)
  self.assertEqual([],list((self.root/'papers').iterdir()))                      # no paper folder, no index.html
 def test_intake_limit_counts_new_folders_across_subfolders(self):
  inbox=self.root/'INBOX'
  for rel in ('00_PRIORITY/u.md','SERIES/S/01 a.md','SERIES/S/02 b.md','GROUPS/G/c.md','d.md'):
   f=inbox/rel; f.parent.mkdir(parents=True,exist_ok=True); f.write_text(rel)
  first=intake(inbox,limit=2); self.assertEqual(2,len(set(first)))
  self.assertIn('u.md',json.loads((first[0]/'paper.json').read_text())['source_file'])  # priority first
  second=intake(inbox,limit=2); self.assertEqual(4,len(set(second)))                 # 2 existing + 2 new

class SiteTests(Base):
 def test_real_claims_template_is_read_and_item_builds_keep_the_full_index(self):
  a=self.paper('a'); b=self.paper('b','Second paper.')
  sha=json.loads((a/'paper.json').read_text())['source_hash']
  filled=(HOME/'templates'/'OUTPUT'/'CLAIMS_LAYER_TEMPLATE_v0.3.md').read_text().replace('{{source_sha256}}',sha).replace('| Total claims | {{count}} |','| Total claims | 7 |')
  run=a/'02_RUNS'/'49_CLAIMS_LAYER'/'20261020T000000Z'; run.mkdir(parents=True); (run/'49_CLAIMS_LAYER.md').write_text(filled)
  site.build(papers=[a])
  rows=(self.root/'papers'/'_SITE'/'index.tsv').read_text().splitlines()
  self.assertEqual(3,len(rows))                                                   # header + both papers
  cols=rows[0].split('\t'); row_a=next(dict(zip(cols,r.split('\t'))) for r in rows[1:] if sha in r)
  self.assertEqual('7',row_a['total_claims'])
  self.assertTrue((a/'index.html').is_file()); self.assertFalse((b/'index.html').is_file())

if __name__=='__main__': unittest.main()
