#!/usr/bin/env python3
"""Lists the English text AAE shows or says, as a gettext template.

    translations/extract.py > translations/aae.pot
    translations/extract.py --update translations/fr.po

Reads the Mac app's Swift and the Rust core, the bindings and the Windows app,
and keeps string literals that read as text for people: not log lines, shell
commands, paths or identifiers. Swift's \\(expression) becomes {name}, which
the translation keeps; Rust's {name} and {} stay as they are.

With --update, adds the new English text to a catalog, keeps the
translations it has, and marks text the code no longer has as obsolete (#~).
"""
import io
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SWIFT = ["macos/Sources/AAE"]
RUST = ["crates/aae-core/src", "crates/aae-ffi/src", "crates/aae-windows/src", "crates/aae-cli/src"]
SKIP_FILES = {"L10n.swift", "l10n.rs", "keyboard_layouts_table.rs"}

LOG_MACRO = re.compile(r"\b(tracing::)?(trace|debug|info|warn|error|event|span)!\s*\($")
SHELLISH = re.compile(
    r"^(pm|am|cmd|settings|dumpsys|getprop|setprop|input|wm|svc|appops|logcat|content|"
    r"monkey|sh|su|ls|rm|cat|echo|mkdir|chmod|killall|kill|reboot|uiautomator|screencap|"
    r"screenrecord|ime|locksettings|device_config|setenforce|ps|grep|test|cp|mv|stat)\b"
    r"|^-{1,2}\w|^\w+=|^[\w.]+/[\w.$]+$|^[\w-]+\.(apk|xml|json|toml|txt|png|wav|gpx|html|zip|ini|img|qcow2)$"
    r"|^https?://|^[\w.]+\.[\w.]+$|\$\{|\$\(|&&|\|\||;\s*\w+="
)


def literals_swift(src):
    """(line, text) of each string literal, with \\(…) as {name}."""
    out = []
    i, n = 0, len(src)
    line = 1
    while i < n:
        c = src[i]
        if c == "\n":
            line += 1
        if src.startswith("//", i):
            j = src.find("\n", i)
            i = n if j < 0 else j
            continue
        if src.startswith("/*", i):
            j = src.find("*/", i)
            line += src[i:j].count("\n")
            i = j + 2
            continue
        if src.startswith('"""', i):
            j = src.find('"""', i + 3)
            body = src[i + 3 : j]
            line += body.count("\n")
            i = j + 3
            continue
        if c == '"':
            text, j = swift_string(src, i)
            out.append((line, text))
            line += src[i:j].count("\n")
            i = j
            continue
        i += 1
    return out


def swift_string(src, i):
    parts = []
    names = {}
    j = i + 1
    while j < len(src):
        c = src[j]
        if c == "\\" and src[j + 1] == "(":
            depth, k = 1, j + 2
            while depth:
                if src[k] == '"':
                    _, k = swift_string(src, k)
                    continue
                depth += {"(": 1, ")": -1}.get(src[k], 0)
                k += 1
            expr = src[j + 2 : k - 1].strip()
            parts.append("{" + slot_name(expr, names) + "}")
            j = k
            continue
        if c == "\\":
            esc = src[j + 1]
            if esc == "u" and src[j + 2] == "{":
                close = src.index("}", j)
                parts.append(chr(int(src[j + 3 : close], 16)))
                j = close + 1
                continue
            parts.append({"n": "\n", "t": "\t", '"': '"', "\\": "\\"}.get(esc, esc))
            j += 2
            continue
        if c == '"':
            return "".join(parts), j + 1
        parts.append(c)
        j += 1
    return "".join(parts), j


def slot_name(expr, names):
    base = expr
    if re.fullmatch(r"[\w.?!$]+", expr):
        base = re.sub(r"[?!$]", "", expr).replace(".", "_").strip("_")
    else:
        base = ""
    base = base or "value"
    count = names.get(base, 0) + 1
    names[base] = count
    return base if count == 1 else f"{base}{count}"


def literals_rust(src):
    """(line, text, end) of each string literal outside tests and log
    macros; end is just after its closing quote."""
    src = without_tests(src)
    out = []
    i, n, line = 0, len(src), 1
    while i < n:
        c = src[i]
        if c == "\n":
            line += 1
            i += 1
            continue
        if src.startswith("//", i):
            j = src.find("\n", i)
            i = n if j < 0 else j
            continue
        if src.startswith("/*", i):
            j = src.find("*/", i)
            line += src[i:j].count("\n")
            i = j + 2
            continue
        if c == "'" and i + 2 < n and (src[i + 2] == "'" or src[i + 1] == "\\"):
            # A character such as 'x', '"' or '\''.
            j = src.find("'", i + 3 if src[i + 1] == "\\" else i + 2)
            i = j + 1
            continue
        m = re.match(r'b?r(#*)"', src[i:])
        if m and (i == 0 or not (src[i - 1].isalnum() or src[i - 1] == "_")):
            hashes = m.group(1)
            end = src.find('"' + hashes, i + len(m.group(0)))
            text = src[i + len(m.group(0)) : end]
            out.append((line, text, in_log(src, i), end + 1 + len(hashes)))
            line += text.count("\n")
            i = end + 1 + len(hashes)
            continue
        if c == '"':
            # Byte strings, b"…", are read and left out.
            byte = i > 0 and src[i - 1] == "b" and not (i > 1 and (src[i - 2].isalnum() or src[i - 2] == "_"))
            j = i + 1
            parts = []
            while src[j] != '"':
                if src[j] == "\\":
                    esc = src[j + 1]
                    if esc == "\n":
                        j += 2
                        while src[j] in " \t\n":
                            j += 1
                        continue
                    parts.append({"n": "\n", "r": "\r", "t": "\t", '"': '"', "\\": "\\", "'": "'", "0": ""}.get(esc, esc))
                    j += 2
                    continue
                parts.append(src[j])
                j += 1
            text = "".join(parts)
            if not byte:
                out.append((line, text, in_log(src, i), j + 1))
            line += src[i:j].count("\n")
            i = j + 1
            continue
        i += 1
    return [(l, t, end) for l, t, log, end in out if not log]


# A part filled in holds a message of its own, translated too, when what
# fills it is named as one: e, why, error, status.detail,
# error.localizedDescription… Anything else, such as a device's, app's or
# file's name, or a program's output, is kept as it is.
MESSAGE_WORDS = {
    "e", "err", "error", "why", "reason", "message", "msg", "detail", "details",
    "problem", "cause", "failure", "localizedDescription",
}
# Calls that keep what a value is.
CONVERSIONS = {
    "to_string", "as_str", "as_ref", "as_deref", "trim", "clone", "to_owned",
    "root_cause", "into", "display",
}


def is_message(expr):
    words = [w for w in re.findall(r"[A-Za-z_]\w*", expr) if w not in CONVERSIONS]
    return bool(words) and words[-1] in MESSAGE_WORDS


def is_message_name(name):
    """For a Swift part's name, such as error_localizedDescription."""
    return is_message(re.sub(r"\d+$", "", name).replace("_", " "))


def format_args(src, i):
    """The arguments after a format string ending at i, as in
    format!("{}: {}", name, e): (positional, {name: expression})."""
    n = len(src)
    while i < n and src[i] in " \t\n\r":
        i += 1
    if i >= n or src[i] != ",":
        return [], {}
    args, depth, start, i = [], 0, i + 1, i + 1
    while i < n:
        c = src[i]
        if c == '"':
            i += 1
            while i < n and src[i] != '"':
                i += 2 if src[i] == "\\" else 1
        elif c == "'" and i + 2 < n and (src[i + 2] == "'" or src[i + 1] == "\\"):
            i = src.find("'", i + 3 if src[i + 1] == "\\" else i + 2)
        elif c in "([{":
            depth += 1
        elif c in ")]}":
            if depth == 0:
                break
            depth -= 1
        elif c == "," and depth == 0:
            args.append(src[start:i].strip())
            start = i + 1
        i += 1
    args.append(src[start:i].strip())
    positional, named = [], {}
    for arg in filter(None, args):
        m = re.fullmatch(r"([A-Za-z_]\w*)\s*=(?!=)\s*(.*)", arg, re.S)
        if m:
            named[m.group(1)] = m.group(2)
        else:
            positional.append(arg)
    return positional, named


def rust_messages(text, src, end):
    """The keys of a Rust format string's parts that hold messages, as
    aae_core::l10n numbers them: a name, or the position of an unnamed one."""
    positional, named = format_args(src, end)
    keys, position, i = set(), 0, 0
    while i < len(text):
        if text.startswith("{{", i) or text.startswith("}}", i):
            i += 2
            continue
        if text[i] == "{":
            close = text.find("}", i)
            if close < 0:
                break
            name = text[i + 1 : close].split(":")[0]
            if re.fullmatch(r"\w*", name):
                if not name:
                    key, k = str(position), position
                    position += 1
                    expr = positional[k] if k < len(positional) else ""
                elif name.isdigit():
                    key = name
                    expr = positional[int(name)] if int(name) < len(positional) else ""
                else:
                    key, expr = name, named.get(name, name)
                if is_message(expr):
                    keys.add(key)
            i = close + 1
            continue
        i += 1
    return keys


def without_tests(src):
    """The source with its test modules blanked out, keeping line numbers."""
    for m in reversed(list(re.finditer(r"#\[cfg\(test\)\]\s*mod \w+\s*\{", src))):
        depth, j = 0, m.end() - 1
        while j < len(src):
            if src[j] == "{":
                depth += 1
            elif src[j] == "}":
                depth -= 1
                if depth == 0:
                    break
            j += 1
        body = src[m.start() : j + 1]
        src = src[: m.start()] + "\n" * body.count("\n") + src[j + 1 :]
    return src


def in_log(src, i):
    """Whether the literal at i is an argument of a tracing macro."""
    start = max(0, i - 400)
    before = src[start:i]
    depth = 0
    for k in range(len(before) - 1, -1, -1):
        ch = before[k]
        if ch == ")":
            depth += 1
        elif ch == "(":
            if depth == 0:
                return bool(LOG_MACRO.search(before[: k + 1]))
            depth -= 1
        elif ch == ";" and depth == 0:
            return False
    return False


def for_people(text):
    t = text.strip()
    if len(t) < 2 or not re.search(r"[A-Za-z]", t):
        return False
    if SHELLISH.search(t):
        return False
    if re.fullmatch(r"[a-z0-9_.\-:/]+", t):  # identifiers, keys, packages
        return False
    if re.fullmatch(r"[A-Z0-9_]+", t):  # constants, env names
        return False
    if re.fullmatch(r"\{[\w.]*\}", t) or re.fullmatch(r"[{}%@\s\w]*", t) and " " not in t and not t[0].isupper():
        return False
    if t.startswith(("io.github", "com.", "android.", "org.", "net.")):
        return False
    if re.search(r"[<>]\w+[>/]|\\[dws]|\[\^|\(\?", t):  # markup, regexes
        return False
    # Text for people: a word with a capital, or a phrase.
    return bool(re.search(r"[A-Z][a-z]", t) or (" " in t and re.search(r"[a-z]{3}", t)))


# Short words the code puts into other text, which the rules above leave out
# as they look like identifiers: states, parts of lists, element types.
INSERTED = [
    "on", "off", "running", "stopped", "starting", "charging", "granted",
    "revoked", "allowed", "passed", "warning", "failed", "saved", "restored",
    "deleted", "cloned", "exported", "changed", "watch", "tv", "megabytes",
    "pixels", "an ARM", "an ARM one", "{} and {}", "Using the Android SDK at {}.",
    "button", "checkbox", "switch", "toggle button", "edit box", "image",
    "slider", "progress bar", "tabs", "list", "web view", "OK", "Warning",
    "Problem", "16 KB pages",
]


def collect():
    """{text: [where]}, and {text: keys of its parts that hold messages}."""
    found = {text: ["translations/extract.py:INSERTED"] for text in INSERTED}
    messages = {}
    for folder in SWIFT + RUST:
        base = os.path.join(ROOT, folder)
        for dirpath, dirs, files in os.walk(base):
            dirs[:] = [d for d in dirs if d != "Generated"]
            for name in sorted(files):
                if name in SKIP_FILES or not name.endswith((".swift", ".rs")):
                    continue
                path = os.path.join(dirpath, name)
                rel = os.path.relpath(path, ROOT)
                with open(path, encoding="utf-8") as f:
                    src = f.read()
                if name.endswith(".swift"):
                    items = [
                        (line, text, {k for k in re.findall(r"\{(\w+)\}", text) if is_message_name(k)})
                        for line, text in literals_swift(src)
                    ]
                else:
                    items = [(line, text, rust_messages(text, src, end)) for line, text, end in literals_rust(src)]
                for line, text, keys in items:
                    if for_people(text):
                        found.setdefault(text, []).append(f"{rel}:{line}")
                        messages.setdefault(text, set()).update(keys)
                # A word with more than one meaning, translated by its use:
                # tr("Watch", context: "…") in Swift, tr_in("…", "Watch") in Rust.
                for m in CONTEXT.finditer(src):
                    context, text = (m.group(2), m.group(1)) if m.group(1) else (m.group(3), m.group(4))
                    line = src.count("\n", 0, m.start()) + 1
                    found.setdefault(f"{context}\x04{text}", []).append(f"{rel}:{line}")
                # Formats filled with names, as in say_with!("{}: {}", …):
                # taken even with few words, for their punctuation.
                for m in FORMAT.finditer(src):
                    text = m.group(1).replace('\\"', '"').replace("\\n", "\n")
                    line = src.count("\n", 0, m.start()) + 1
                    found.setdefault(text, []).append(f"{rel}:{line}")
    return found, messages


FORMAT = re.compile(r'\b(?:say_with!|tr_format)\(\s*"((?:[^"\\]|\\.)*)"')

CONTEXT = re.compile(
    r'\btr\("((?:[^"\\]|\\.)*)", context: "([^"]*)"\)'
    r'|\btr_in\("([^"]*)", "((?:[^"\\]|\\.)*)"\)'
)


def quote(text):
    s = text.replace("\\", "\\\\").replace('"', '\\"').replace("\t", "\\t").replace("\r", "\\r")
    if "\n" in s:
        lines = s.split("\n")
        body = "\n".join('"' + l + ('\\n"' if k < len(lines) - 1 else '"') for k, l in enumerate(lines))
        return '""\n' + body
    return '"' + s + '"'


def read_po(path):
    """{msgid: msgstr} and the header."""
    sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
    from po import entries

    with open(path, encoding="utf-8") as f:
        return dict(entries(f.read()))


HEADER = '''msgid ""
msgstr ""
"Content-Type: text/plain; charset=UTF-8\\n"
"Language: {lang}\\n"
'''


def write(found, messages, translations, lang, out):
    out.write("# English text from AAE, with its translation. Each msgid is the English\n")
    out.write("# text as the code writes it; {name} parts stay as they are.\n")
    out.write("# Follow translations/guide-fr.md for terms, tone and typography.\n")
    out.write(HEADER.format(lang=lang))
    def place(text):
        path, _, line = found[text][0].rpartition(":")
        return (path, int(line) if line.isdigit() else 0)

    for text in sorted(found, key=place):
        out.write("\n")
        refs = found[text]
        out.write("#: " + " ".join(refs[:4]) + (" …" if len(refs) > 4 else "") + "\n")
        if messages.get(text):
            out.write("#. messages: " + ", ".join(sorted(messages[text])) + "\n")
        context, _, english = text.rpartition("\x04")
        if context:
            out.write("msgctxt " + quote(context) + "\n")
        out.write("msgid " + quote(english) + "\n")
        out.write("msgstr " + quote(translations.get(text, "")) + "\n")
    obsolete = [t for t in translations if t and t not in found and translations[t]]
    for text in sorted(obsolete):
        out.write("\n#~ msgid " + quote(text).replace("\n", "\n#~ ") + "\n")
        out.write("#~ msgstr " + quote(translations[text]).replace("\n", "\n#~ ") + "\n")


def main():
    found, messages = collect()
    if len(sys.argv) > 2 and sys.argv[1] == "--update":
        path = sys.argv[2]
        translations = read_po(path) if os.path.exists(path) else {}
        translations.pop("", None)
        lang = os.path.splitext(os.path.basename(path))[0]
        # Written whole at the end, so a failure leaves the catalog as it was.
        out = io.StringIO()
        write(found, messages, translations, lang, out)
        with open(path, "w", encoding="utf-8") as f:
            f.write(out.getvalue())
        done = sum(1 for t in found if translations.get(t))
        print(f"{len(found)} texts, {done} translated, {len(found) - done} to translate.", file=sys.stderr)
    else:
        write(found, messages, {}, "", sys.stdout)


if __name__ == "__main__":
    main()
