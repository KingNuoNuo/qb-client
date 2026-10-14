"""无 Rust 工具链情况下的静态自检。

能查的：
  1. 括号/中括号/花括号配平（跳过字符串、字符、注释）
  2. 同一文件内重复的 fn / struct / enum / const / type 定义
  3. `w::` / `th::` 引用的符号在对应模块里是否真的存在
  4. app.rs 里每个 `fn` 体内用到 `p.` 的，是否绑定了 `p`
  5. 残留的旧常量 C_DL / C_UP / C_ERR / C_WARN / C_DIM

查不了的：类型是否正确、trait 是否满足、借用是否成立 —— 那些只有 rustc 知道。
"""

import io
import os
import re
import sys

ROOT = os.path.dirname(os.path.abspath(__file__))
SRC = os.path.join(ROOT, "src")

problems = []


def read(p):
    with io.open(p, encoding="utf-8") as f:
        return f.read()


def strip_code(text):
    """把字符串/字符/注释替换成等长空白，保留下标与换行，便于后续正则定位。"""
    out = list(text)
    i = 0
    n = len(text)
    while i < n:
        c = text[i]
        if c == '"':
            out[i] = " "
            i += 1
            while i < n:
                if text[i] == "\\":
                    out[i] = " "
                    if i + 1 < n:
                        out[i + 1] = " "
                    i += 2
                    continue
                if text[i] == '"':
                    out[i] = " "
                    i += 1
                    break
                if text[i] != "\n":
                    out[i] = " "
                i += 1
        elif c == "/" and i + 1 < n and text[i + 1] == "/":
            while i < n and text[i] != "\n":
                out[i] = " "
                i += 1
        elif c == "/" and i + 1 < n and text[i + 1] == "*":
            out[i] = " "
            out[i + 1] = " "
            i += 2
            while i < n:
                if text[i] == "*" and i + 1 < n and text[i + 1] == "/":
                    out[i] = " "
                    out[i + 1] = " "
                    i += 2
                    break
                if text[i] != "\n":
                    out[i] = " "
                i += 1
        elif c == "'":
            # 字符字面量 vs 生命周期
            if i + 2 < n and text[i + 2] == "'":
                for k in range(i, i + 3):
                    out[k] = " "
                i += 3
            elif i + 3 < n and text[i + 1] == "\\" and text[i + 3] == "'":
                for k in range(i, i + 4):
                    out[k] = " "
                i += 4
            else:
                i += 1
        else:
            i += 1
    return "".join(out)


FILES = ["main.rs", "app.rs", "theme.rs", "fonts.rs", "widgets.rs",
         "api.rs", "model.rs", "util.rs", "worker.rs", "config.rs"]

raw = {}
code = {}
for name in FILES:
    p = os.path.join(SRC, name)
    if not os.path.exists(p):
        problems.append("缺少文件: " + name)
        continue
    raw[name] = read(p)
    code[name] = strip_code(raw[name])

# ---------- 1. 括号配平 ----------
PAIRS = {")": "(", "]": "[", "}": "{"}
for name in FILES:
    if name not in code:
        continue
    stack = []
    for idx, ch in enumerate(code[name]):
        if ch in "([{":
            stack.append((ch, idx))
        elif ch in ")]}":
            if not stack:
                line = code[name][:idx].count("\n") + 1
                problems.append("%s:%d 多余的 '%s'" % (name, line, ch))
                break
            open_ch, open_idx = stack.pop()
            if open_ch != PAIRS[ch]:
                line = code[name][:idx].count("\n") + 1
                problems.append(
                    "%s:%d '%s' 与第 %d 行的 '%s' 不匹配"
                    % (name, line, ch, code[name][:open_idx].count("\n") + 1, open_ch)
                )
                break
    else:
        if stack:
            line = code[name][:stack[-1][1]].count("\n") + 1
            problems.append("%s:%d 未闭合的 '%s'" % (name, line, stack[-1][0]))

# ---------- 2. 重复定义 ----------
# 注意要按「所属 impl 块」区分：`impl Default for A { fn default }` 和
# `impl Default for B { fn default }` 同文件出现是完全合法的，不能算重复。
DEF_RE = re.compile(
    r"^\s*(?:pub(?:\([^)]*\))?\s+)?(fn|struct|enum|trait|type|const|static)\s+([A-Za-z_][A-Za-z0-9_]*)",
    re.M,
)


def impl_spans(text):
    """粗略切出所有 `impl ... { ... }` 的区间，返回 [(起, 止, 头部文本)]。"""
    spans = []
    for m in re.finditer(r"\bimpl\b[^{;]*\{", text):
        start = m.end() - 1
        depth = 0
        j = start
        while j < len(text):
            if text[j] == "{":
                depth += 1
            elif text[j] == "}":
                depth -= 1
                if depth == 0:
                    break
            j += 1
        spans.append((start, j, text[m.start(): m.end() - 1].strip()))
    return spans


def has_cfg_attr(text, pos):
    """定义上方紧邻的属性里是否带 `#[cfg(...)`。

    `#[cfg(windows)] fn f` + `#[cfg(not(windows))] fn f` 是**合法的平台配对**，
    不能算重复 —— 所以带 cfg 门住的定义直接跳过这项检查。
    往上扫时要跳过文档注释（`///`）和普通注释，遇到第一行"别的"就停。
    """
    for line in reversed(text[:pos].splitlines()[-8:]):
        s = line.strip()
        if not s or s.startswith("///") or s.startswith("//"):
            continue
        return s.startswith("#[cfg(")
    return False


for name in FILES:
    if name not in code:
        continue
    spans = impl_spans(code[name])

    def owner(pos, spans=spans):
        for s, e, head in spans:
            if s <= pos <= e:
                return head
        return None

    seen = {}
    for m in DEF_RE.finditer(code[name]):
        kind, ident = m.group(1), m.group(2)
        own = owner(m.start())
        if has_cfg_attr(code[name], m.start()):
            continue  # cfg 门住的平台配对是合法的
        key = (own + "::" + ident) if own else ident
        seen.setdefault(key, []).append(
            (kind, code[name][: m.start()].count("\n") + 1)
        )
    for key, hits in seen.items():
        if len(hits) > 1 and hits[0][0] != "static":
            problems.append(
                "%s: '%s' 定义了 %d 次 —— %s"
                % (name, key, len(hits), ", ".join("第%d行(%s)" % (ln, k) for k, ln in hits))
            )

# ---------- 3. 跨模块符号 ----------
MOD_DEFS = {}
for mod, path in (("w", "widgets.rs"), ("th", "theme.rs")):
    if path not in code:
        continue
    names = set()
    for m in re.finditer(
        r"^\s*pub\s+(?:fn|struct|enum|trait|type|const|static)\s+([A-Za-z_][A-Za-z0-9_]*)",
        code[path],
        re.M,
    ):
        names.add(m.group(1))
    MOD_DEFS[mod] = names

for name in FILES:
    if name not in code:
        continue
    for m in re.finditer(r"\b(w|th)::([A-Za-z_][A-Za-z0-9_]*)", code[name]):
        mod, ident = m.group(1), m.group(2)
        if mod in MOD_DEFS and ident not in MOD_DEFS[mod]:
            line = code[name][: m.start()].count("\n") + 1
            problems.append("%s:%d 引用了不存在的 %s::%s" % (name, line, mod, ident))

# ---------- 4. app.rs 里 p 的作用域 ----------
if "app.rs" in code:
    c = code["app.rs"]
    # 逐个 fn 切出正文
    starts = [m.start() for m in re.finditer(r"^\s{0,8}fn\s+[A-Za-z_]", c, re.M)]
    starts.append(len(c))
    for i in range(len(starts) - 1):
        seg = c[starts[i]: starts[i + 1]]
        header = raw["app.rs"][starts[i]: starts[i] + 140].split("\n")[0]
        body_start = seg.find("{")
        if body_start < 0:
            continue
        head, body = seg[:body_start], seg[body_start:]
        uses_p = re.search(r"\bp\s*\.", body) or re.search(r"\(\s*p\s*,", body)
        bound = re.search(r"\blet\s+p\s*=", head + body) or re.search(r"\bp\s*:\s*&Palette", head)
        if uses_p and not bound:
            line = c[: starts[i]].count("\n") + 1
            problems.append("app.rs:%d 函数用到 p 但没绑定: %s" % (line, header.strip()[:70]))

# ---------- 5. 旧常量残留 ----------
for name in FILES:
    if name not in code:
        continue
    for old in ("C_DL", "C_UP", "C_ERR", "C_WARN", "C_DIM", "setup_fonts"):
        for m in re.finditer(r"\b" + old + r"\b", code[name]):
            line = code[name][: m.start()].count("\n") + 1
            problems.append("%s:%d 残留旧符号 %s" % (name, line, old))

# ---------- 报告 ----------
print("检查文件: %s" % ", ".join(f for f in FILES if f in code))
if problems:
    print("\n发现 %d 个问题:" % len(problems))
    for p in problems:
        print("  - " + p)
    sys.exit(1)
print("\n静态检查全部通过")
