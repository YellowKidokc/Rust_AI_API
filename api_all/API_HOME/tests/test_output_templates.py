import json,os,sys,tempfile,threading,unittest
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
from pathlib import Path
from unittest.mock import patch
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from engine import paper_station
from engine.papers import create

class _Echo(BaseHTTPRequestHandler):
 """Plays a model that hands the template back untouched."""
 prompts=[]
 def do_POST(self):
  prompt=json.loads(self.rfile.read(int(self.headers['Content-Length'])))['messages'][-1]['content']; _Echo.prompts.append(prompt)
  reply=prompt.split('OUTPUT TEMPLATE:\n',1)[-1].split('\n\nSOURCE DOCUMENT:',1)[0]
  out=json.dumps({'choices':[{'message':{'content':reply}}],'usage':{'prompt_tokens':7,'completion_tokens':3}}).encode()
  self.send_response(200); self.send_header('Content-Type','application/json'); self.send_header('Content-Length',str(len(out))); self.end_headers(); self.wfile.write(out)
 def log_message(self,*_): pass

class OutputTemplateTests(unittest.TestCase):
 def setUp(self):
  self.server=ThreadingHTTPServer(('127.0.0.1',0),_Echo); threading.Thread(target=self.server.serve_forever,daemon=True).start()
  self.td=tempfile.TemporaryDirectory(); root=Path(self.td.name); (root/'papers').mkdir()
  (root/'paths.json').write_text(json.dumps({'papers_root':str(root/'papers'),'catalog_db':''}))
  self.env={'ONE_MENU_PATHS_FILE':str(root/'paths.json'),'FORGE_URL':f'http://127.0.0.1:{self.server.server_port}','FORGE_ROUTE':'test'}
  src=root/'p.md'; src.write_text('Grace restores order.'); self.root=root
  with patch.dict(os.environ,self.env): self.paper=create(src)
  self.meta=json.loads((self.paper/'paper.json').read_text()); _Echo.prompts.clear()
 def tearDown(self): self.server.shutdown(); self.server.server_close(); self.td.cleanup()
 def run_station(self,*extra):
  with patch.dict(os.environ,self.env), patch.object(sys,'argv',['x',str(self.paper),*extra]):
   paper_station.run_ai('49_CLAIMS_LAYER',[('claims','Build the claims layer.')])
 def latest_md(self): return sorted((self.paper/'02_RUNS'/'49_CLAIMS_LAYER').glob('*/49_CLAIMS_LAYER.md'))[-1].read_text()

 def test_station_default_template_with_code_filled_fields(self):
  self.run_station()
  prompt=_Echo.prompts[0]
  self.assertIn('OUTPUT TEMPLATE:',prompt); self.assertIn('## Hidden premises',prompt)
  self.assertIn(f"source_sha256: {self.meta['source_hash']}",prompt)    # filled before the call
  self.assertIn('Grace restores order.',prompt)                            # whole paper still sent
  md=self.latest_md()
  self.assertIn('semantic_model: "deepseek-chat"',md)                     # filled after the call
  self.assertIn('"total": 10',md); self.assertNotIn('{{provider}}',md)

 def test_override_template_and_rerun_rules(self):
  bullets=self.root/'bullets.md'; bullets.write_text('- Claim: {{claim}}\n- Model: {{exact_model_string}}\n')
  self.run_station(); self.run_station()
  self.assertEqual(1,len(_Echo.prompts))                                   # same template: skipped
  self.run_station('--template',str(bullets))
  self.assertEqual(2,len(_Echo.prompts))                                   # new template: runs again
  self.assertEqual('- Claim: {{claim}}\n- Model: deepseek-chat',self.latest_md().strip())

 def test_missing_template_is_an_error_not_a_blank(self):
  with patch.dict(os.environ,self.env): self.assertRaises(FileNotFoundError,paper_station.output_template,'49_CLAIMS_LAYER','NO_SUCH_TEMPLATE')

if __name__=='__main__': unittest.main()
