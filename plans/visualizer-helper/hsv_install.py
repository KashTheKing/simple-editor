# Put a generated Luau script into Simple Editor's scripts folder with its ids filled in, so it runs with
# scripts.run {name} (no need to paste 50+ KB of keys through the MCP call).
#
#   python hsv_install.py OUT_DIR/keys.lua hsv-keys CLIP=7
#   python hsv_install.py OUT_DIR/lyrics.lua hsv-lyrics VIS=7 TRACK=3
#   python hsv_install.py OUT_DIR/shaders.lua hsv-shaders VIS=7
#
# Plain script runs get a fixed 5 s budget; every generated script stays under it (split it if a much
# longer song pushes keys.lua over).
import os, re, sys
src, name, *sets = sys.argv[1:]
s = open(src, encoding='utf8').read()
for kv in sets:
    k, v = kv.split('=', 1)
    s, n = re.subn(r'^local %s = [^\n-]*' % re.escape(k), 'local %s = %s ' % (k, v), s, count=1, flags=re.M)
    assert n == 1, 'no "local %s =" line in %s' % (k, src)
d = os.path.join(os.environ['APPDATA'], 'SimpleEditor-Dev', 'scripts')
if not os.path.isdir(d): d = os.path.join(os.environ['APPDATA'], 'SimpleEditor', 'scripts')
os.makedirs(d, exist_ok=True)
p = os.path.join(d, name + '.luau')
# one edit batch: each tool call is then not its own undo step (much faster, so a run fits its 5 s)
s = ('-- @name %s\neditor.tool("edit.begin", {label = "%s"})\nlocal function main()\n' % (name, name) + s +
     '\nend\nlocal ok, res = pcall(main)\neditor.tool("edit.end", {})\nif not ok then error(res) end\nreturn res\n')
open(p, 'w', encoding='utf8', newline='\n').write(s)
print(p)
