"""Reads a gettext PO catalog, for the translators' scripts.

aae_core::l10n reads the same catalogs in AAE itself.
"""


def unquote(text):
    inner = text.strip()
    if inner.startswith('"') and inner.endswith('"'):
        inner = inner[1:-1]
    out = []
    i = 0
    while i < len(inner):
        c = inner[i]
        if c == "\\" and i + 1 < len(inner):
            nxt = inner[i + 1]
            out.append({"n": "\n", "r": "\r", "t": "\t"}.get(nxt, nxt))
            i += 2
            continue
        out.append(c)
        i += 1
    return "".join(out)


def entries(po):
    """(msgid, msgstr) of each entry; with a msgctxt, the msgid is
    "context\x04msgid", as gettext writes it."""
    msgid = msgstr = context = None
    field = None

    def key():
        return msgid if context is None else f"{context}\x04{msgid}"

    for raw in po.splitlines():
        line = raw.strip()
        if not line:
            if msgid is not None:
                yield key(), msgstr or ""
            msgid = msgstr = field = context = None
            continue
        if line.startswith("#"):
            continue
        if line.startswith("msgctxt "):
            if msgid is not None:
                yield key(), msgstr or ""
                msgid = msgstr = None
            context, field = unquote(line[8:]), "ctxt"
        elif line.startswith("msgid "):
            if msgid is not None:
                yield key(), msgstr or ""
                context = None
            msgid, msgstr, field = unquote(line[6:]), None, "id"
        elif line.startswith("msgstr "):
            msgstr, field = unquote(line[7:]), "str"
        elif line.startswith('"'):
            if field == "id":
                msgid += unquote(line)
            elif field == "str":
                msgstr += unquote(line)
            elif field == "ctxt":
                context += unquote(line)
    if msgid is not None:
        yield key(), msgstr or ""
