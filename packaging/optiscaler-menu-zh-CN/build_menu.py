"""Compile a fixed, validated translation table and a small embedded CJK font."""
import argparse
import hashlib
import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parent
PRINTF = re.compile(r"%(?:%|[-+ #0']*(?:\*|\d+)?(?:\.(?:\*|\d+))?(?:hh|ll|I64|I32|[hjltzL])?[diouxXfFeEgGaAcspn])")
ESCAPES = {"n": "\n", "r": "\r", "t": "\t", "s": " ", "=": "=", "\\": "\\"}

def unescape(text):
    return re.sub(r"\\([nrts=\\])", lambda m: ESCAPES[m[1]], text)

def load_table():
    table = {}
    for line in (ROOT / "upstream-zh-CN.txt").read_text(encoding="utf-8-sig").splitlines():
        if not line.strip() or line.lstrip().startswith((";", "#")):
            continue
        fields = re.split(r"(?<!\\)=", line, maxsplit=1)
        if len(fields) == 2:
            key, value = [unescape(x.strip()) for x in fields]
            table[key] = value
    for line in (ROOT / "overlay.tsv").read_text(encoding="utf-8-sig").splitlines():
        key, value = line.split("\t")
        table[unescape(key)] = unescape(value)
    for key, value in table.items():
        if not key or not value or "\0" in key + value:
            raise ValueError(f"Empty key/value or NUL: {key!r}")
        if "%n" in value:
            raise ValueError(f"Unsafe printf conversion: {key!r}")
    return table

def main():
    from fontTools import subset
    from fontTools.ttLib import TTFont
    p = argparse.ArgumentParser()
    p.add_argument("font", type=Path)
    p.add_argument("out", type=Path)
    args = p.parse_args()
    font_sha = hashlib.sha256(args.font.read_bytes()).hexdigest()
    if font_sha != "2c76254f6fc379fddfce0a7e84fb5385bb135d3e399294f6eeb6680d0365b74b":
        p.error("Use the pinned Noto Sans CJK SC Regular font")
    args.out.mkdir(parents=True, exist_ok=True)
    table = load_table()
    literal = lambda s: json.dumps(s, ensure_ascii=False)
    def safe_format(k, v):
        return PRINTF.findall(k) == PRINTF.findall(v) and '%' not in PRINTF.sub('',v)
    lines = [f"    {{{literal(k)}, {literal(v)}, {str(safe_format(k,v)).lower()}}}," for k,v in sorted(table.items())]
    (args.out / "MenuZhEntries.inc").write_text("\n".join(lines)+"\n", encoding="utf-8-sig")
    points = sorted({ord(ch) for s in table.values() for ch in s if ord(ch) >= 0x100})
    font = TTFont(args.font, recalcTimestamp=False)
    missing = set(points).difference(font.getBestCmap())
    if missing:
        raise ValueError(f"Font lacks {sorted(missing)}")
    worker = subset.Subsetter()
    worker.populate(unicodes=points)
    worker.subset(font)
    for item in font["name"].names:
        if item.nameID in (1, 3, 4, 6, 16):
            item.string = "RTXManagerMenuSC".encode(item.getEncoding())
    if "CFF " in font:
        top = font["CFF "].cff
        top.fontNames[0] = "RTXManagerMenuSC"
        top.topDictIndex[0].FullName = "RTXManagerMenuSC"
        top.topDictIndex[0].FamilyName = "RTXManagerMenuSC"
    import io
    stream = io.BytesIO()
    font.save(stream)
    data = stream.getvalue()
    ranges = []
    for x in points:
        if ranges and x == ranges[-1]+1: ranges[-1] = x
        else: ranges += [x,x]
    with (args.out / "MenuZhFont.h").open("w",encoding="ascii") as out:
        out.write("#pragma once\nstatic constexpr unsigned short kMenuGlyphs[] = {")
        out.write(",".join(map(str,ranges))+",0};\nstatic const unsigned char kMenuFont[] = {\n")
        for i in range(0,len(data),24): out.write(",".join(map(str,data[i:i+24]))+",\n")
        out.write("};\n")
    print(f"{len(table)} entries, {len(points)} glyphs, {len(data)} font bytes; source font SHA-256 {hashlib.sha256(args.font.read_bytes()).hexdigest()}")

if __name__ == "__main__":
    main()
