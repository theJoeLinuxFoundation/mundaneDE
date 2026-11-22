"""Serve a localhost-only getDisplayMedia page and read the frames received by Firefox."""

from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path
import os
import subprocess
import tempfile
import threading

PAGE = b"""<!doctype html><meta charset=utf-8><title>Portal WebRTC test</title>
<button autofocus id="start">Share a screen</button><video muted autoplay></video>
<script>
const record = text => fetch('/event',{method:'POST',body:text});
start.onclick=async()=>{
  try {
    const stream=await navigator.mediaDevices.getDisplayMedia({video:true,audio:false});
    const track=stream.getVideoTracks()[0]; track.onended=()=>record('ended');
    const video=document.querySelector('video'); video.srcObject=stream;
    await video.play(); const canvas=document.createElement('canvas');
    canvas.width=32;canvas.height=24;const ctx=canvas.getContext('2d',{willReadFrequently:true});
    let last='';
    function frame(){
      ctx.drawImage(video,0,0,32,24);
      const color=(x,y)=>Array.from(ctx.getImageData(x,y,1,1).data).slice(0,3).join(',');
      const colors=color(16,6)+' '+color(16,18);
      if(last!==colors){record('pixels '+colors);last=colors;}
      video.requestVideoFrameCallback(frame);
    }
    video.requestVideoFrameCallback(frame);
  }catch(e){record('denied '+e.name);}
};
</script>"""


class Page(BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200)
        self.send_header("Content-Type", "text/html")
        self.end_headers()
        self.wfile.write(PAGE)

    def do_POST(self):
        message = self.rfile.read(int(self.headers["Content-Length"])).decode()
        print("FIREFOX", message, flush=True)
        self.send_response(200)
        self.end_headers()

    def log_message(self, *_):
        pass


server = HTTPServer(("127.0.0.1", 0), Page)
threading.Thread(target=server.serve_forever, daemon=True).start()
with tempfile.TemporaryDirectory(dir=os.environ["XDG_RUNTIME_DIR"]) as profile:
    (
        Path(profile) / "user.js"
    ).write_text("""user_pref("browser.aboutwelcome.enabled",false);
user_pref("browser.shell.checkDefaultBrowser",false);
user_pref("browser.startup.firstrunSkipsHomepage",true);
user_pref("browser.sessionstore.resume_from_crash",false);
user_pref("media.webrtc.capture.allow-pipewire",true);
user_pref("accessibility.force_disabled",-1);
""")
    browser = subprocess.Popen(
        [
            "firefox",
            "--no-remote",
            "--profile",
            profile,
            f"http://127.0.0.1:{server.server_port}",
        ],
        env={**os.environ, "MOZ_ENABLE_WAYLAND": "1"},
    )
    (
        Path(os.environ["XDG_RUNTIME_DIR"]) / "mundane-portal-test/browser.pid"
    ).write_text(str(browser.pid))
    browser.wait()
