"""Poke the helper by hand: python probe.py <se-html.exe> [page.html]. Prints each answer's head and body size."""
import json, struct, subprocess, sys, time

exe = sys.argv[1]
p = subprocess.Popen([exe], stdin=subprocess.PIPE, stdout=subprocess.PIPE)


def call(head, body=b""):
    h = json.dumps(head).encode()
    msg = struct.pack("<II", 4 + len(h) + len(body), len(h)) + h + body
    p.stdin.write(msg)
    p.stdin.flush()
    total = struct.unpack("<I", p.stdout.read(4))[0]
    buf = p.stdout.read(total)
    hl = struct.unpack("<I", buf[:4])[0]
    return json.loads(buf[4:4 + hl]), buf[4 + hl:]


def show(name, r):
    h, b = r
    s = {k: v for k, v in h.items() if k != "elements"}
    print(f"{name:10s} {json.dumps(s)[:150]}  body={len(b)}")


show("ping", call({"cmd": "ping"}))
page = open(sys.argv[2], encoding="utf-8").read() if len(sys.argv) > 2 else (
    '<html><head><style>html,body{margin:0}.b{position:absolute;left:10px;top:10px;width:80px;height:80px;background:#f00;'
    'transition:background 1s linear;animation:a 1s linear both}.b:hover{background:#00f}@keyframes a{from{opacity:0}to{opacity:1}}'
    'a{position:absolute;left:10px;top:120px;display:block;width:90px;height:30px;background:#888}</style></head>'
    '<body><div class="b"></div><a href="https://example.com/x">go there</a><button style="position:absolute;top:160px">Press</button></body></html>')
h, _ = call({"cmd": "load", "html": page, "base": "", "w": 400, "h": 300, "scale": 1})
show("load", (h, b""))
i = h["id"]
for t in (0.0, 0.5, 1.0):
    t0 = time.time()
    r = call({"cmd": "frame", "id": i, "t": t, "fmt": "png"})
    show(f"frame {t}", r)
    print("   ms", round((time.time() - t0) * 1000))
show("hover", call({"cmd": "frame", "id": i, "t": 1.5, "ptr": [40, 40, 0], "fmt": "rgba"}))
show("frames", call({"cmd": "frames", "id": i, "items": [{"t": 2.0}, {"t": 2.1}, {"t": 2.2}], "fmt": "png"}))
show("elements", call({"cmd": "elements", "id": i}))
print(json.dumps(call({"cmd": "elements", "id": i})[0]["elements"])[:400])
show("region", call({"cmd": "region", "id": i, "rect": [0, 0, 100, 100], "scale": 3, "t": 1, "fmt": "png"}))
bad = '<html><body><div style="width:50px;height:50px;background:red;filter:hue-rotate(90deg)"></div></body></html>'
b = call({"cmd": "load", "html": bad, "w": 100, "h": 100, "scale": 1})
bid = b[0]["id"]
show("panic?", call({"cmd": "frame", "id": bid, "t": 0, "fmt": "png"}))
show("alive", call({"cmd": "frame", "id": i, "t": 0.2, "fmt": "png"}))
show("nopage", call({"cmd": "frame", "id": 999, "t": 0}))
show("unknown", call({"cmd": "wat"}))
show("quit", call({"cmd": "quit"}))
print("exit", p.wait(10))
