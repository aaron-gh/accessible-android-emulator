#!/usr/bin/env python3
"""Checks a French catalog against translations/guide-fr.md.

    translations/check.py translations/fr.po

Prints each problem with its English text. The slots ({name}, %@) are
checked by `cargo test -p aae-core l10n`.
"""
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from po import entries  # noqa: E402

NBSP = "\u00a0\u202f"


def problems(english, french):
    found = []
    if re.search(r"[^\s%s][;:!?](\s|$)" % NBSP, french) and not re.search(r"https?:|\d:\d", french):
        found.append("espace insécable manquante avant ; : ! ?")
    if re.search(r" [;:!?]", french):
        found.append("espace ordinaire avant ; : ! ? (il faut une insécable)")
    if re.search(r"«[^%s]|[^%s]»" % (NBSP, NBSP), french):
        found.append("espace insécable manquante dans « »")
    if '"' in french and '"' not in english:
        found.append('guillemets droits " (il faut « »)')
    if "..." in french:
        found.append("trois points (il faut …)")
    if re.search(r"\bappli(s|cation|cations)?\b", french, re.I):
        found.append("« appli » ou « application » (le guide dit « app »)")
    if re.search(r"\bveuillez\b", french, re.I):
        found.append("« Veuillez » (à éviter)")
    if re.search(r"\b(snapshot|build|wipe|quick boot|cold boot)\b", french, re.I) and not re.search(r"`|aae |--", french):
        found.append("terme anglais resté (voir le glossaire)")
    amp = lambda t: len(re.findall(r"(?<!&)&(?!&)", t))
    if amp(english) != amp(french):
        found.append("touche d'accès & : il en faut autant qu'en anglais")
    if ("\t" in english) != ("\t" in french):
        found.append("tabulation du raccourci perdue")
    if english[:1].isspace() != french[:1].isspace() or english[-1:].isspace() != french[-1:].isspace():
        found.append("espaces au début ou à la fin différents de l'anglais")
    if english.rstrip().endswith(".") != french.rstrip().endswith("."):
        found.append("point final : présent dans une langue et pas dans l'autre")
    if english.count("\n") != french.count("\n"):
        found.append("nombre de lignes différent")
    return found


def main():
    path = sys.argv[1]
    with open(path, encoding="utf-8") as f:
        po = f.read()
    total = 0
    for english, french in entries(po):
        if not english or not french:
            continue
        for problem in problems(english, french):
            total += 1
            print(f"{problem}\n  en : {english!r}\n  fr : {french!r}")
    print(f"{total} problème(s).", file=sys.stderr)
    sys.exit(1 if total else 0)


if __name__ == "__main__":
    main()
