#!/usr/bin/env python3

import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
TIMEOUT_SECONDS = 20 * 60
USAGE = """usage: python3 scripts/kani.py prove|mutations
  prove      every harness holds and reaches every kani::cover!
  mutations  every broken variant in src/**/*.mutations is caught"""

HARNESS_HEADER = re.compile(r"^Checking harness (\S+)\.\.\.$", re.M)
COVERS = re.compile(r"\*\* (\d+) of (\d+) cover properties satisfied")
PROOF_FN = re.compile(r"#\[kani::proof\][^\n]*\n(?:\s*#\[[^\n]*\n)*\s*fn\s+(\w+)")


def kani(*args):
    cmd = ["cargo", "kani", "--lib", *args]
    try:
        done = subprocess.run(
            cmd,
            cwd=ROOT,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            timeout=TIMEOUT_SECONDS,
        )
    except subprocess.TimeoutExpired:
        return None, f"timed out after {TIMEOUT_SECONDS}s: {' '.join(cmd)}"
    return done.returncode, done.stdout


def harness_reports(output):
    marks = list(HARNESS_HEADER.finditer(output))
    reports = {}
    for i, mark in enumerate(marks):
        end = marks[i + 1].start() if i + 1 < len(marks) else len(output)
        reports[mark.group(1)] = output[mark.end() : end]
    return reports


def verdict(report):
    if "VERIFICATION:- FAILED" in report:
        return "fails"
    if "VERIFICATION:- SUCCESSFUL" not in report:
        return "unknown"
    covers = COVERS.search(report)
    if covers and covers.group(1) != covers.group(2):
        return "fails"
    return "holds"


def tail(text, lines=40):
    return "\n".join(text.rstrip().splitlines()[-lines:])


def prove():
    code, output = kani()
    reports = harness_reports(output)
    if not reports:
        print(tail(output))
        print("FAIL no harness ran")
        return 1
    ok = code == 0
    for name, report in sorted(reports.items()):
        covers = COVERS.search(report)
        if verdict(report) == "holds" and covers and int(covers.group(2)) > 0:
            print(f"PASS {name} ({covers.group(1)} of {covers.group(2)} covers)")
            continue
        ok = False
        if covers is None or int(covers.group(2)) == 0:
            print(f"FAIL {name}: no kani::cover! witness")
        else:
            print(f"FAIL {name}")
        print(tail(report))
    print(f"SUMMARY {len(reports)} harnesses, {'all hold' if ok else 'not all hold'}")
    return 0 if ok else 1


def module_path(rs):
    rel = rs.relative_to(ROOT / "src").with_suffix("")
    parts = [p for p in rel.parts if p not in ("lib", "mod")]
    return "::".join(parts)


def parse_mutations(path):
    blocks = re.split(r"\n\s*\n", path.read_text())
    for block in blocks:
        lines = [l for l in block.splitlines() if l and not l.startswith("#")]
        if not lines:
            continue
        label = harness = None
        old, new = [], []
        for line in lines:
            if line.startswith("mutation "):
                label = line[len("mutation ") :].strip()
            elif line.startswith("detects "):
                harness = line[len("detects ") :].strip()
            elif line == "-" or line.startswith("- "):
                old.append(line[2:])
            elif line == "+" or line.startswith("+ "):
                new.append(line[2:])
            else:
                raise SystemExit(f"{path}: cannot read line: {line!r}")
        if not (label and harness and old):
            raise SystemExit(f"{path}: a mutation needs `mutation`, `detects` and `-` lines")
        yield label, harness, "\n".join(old) + "\n", "\n".join(new) + "\n" if new else ""


def mutations():
    files = sorted((ROOT / "src").rglob("*.mutations"))
    if not files:
        print("FAIL no .mutations files under src/")
        return 1
    ok = True
    counts = {"DETECTED": 0, "MISSED": 0, "ERROR": 0}
    for mfile in files:
        rs = mfile.with_suffix(".rs")
        original = rs.read_text()
        prefix = module_path(rs)
        harnesses = {
            f"{prefix}::proofs::{name}" if prefix else f"proofs::{name}"
            for name in PROOF_FN.findall(original)
        }
        named = set()
        for label, harness, old, new in parse_mutations(mfile):
            named.add(harness)
            where = f"{mfile.relative_to(ROOT)}: {label}"
            if original.count(old) != 1:
                print(f"ERROR {where}: the `-` lines must occur exactly once in {rs.name}")
                counts["ERROR"] += 1
                ok = False
                continue
            try:
                rs.write_text(original.replace(old, new, 1))
                code, output = kani("--harness", harness, "--exact")
            finally:
                rs.write_text(original)
            report = harness_reports(output).get(harness, "")
            result = verdict(report) if code is not None else "unknown"
            if result == "fails":
                status = "DETECTED"
            elif result == "holds":
                status = "MISSED"
            else:
                status = "ERROR"
            counts[status] += 1
            print(f"{status} {where} ({harness})")
            if status != "DETECTED":
                ok = False
                print(tail(report or output))
        for harness in sorted(harnesses - named):
            print(f"UNCOVERED {harness}: no mutation in {mfile.relative_to(ROOT)} names it")
            ok = False
    print(
        "SUMMARY {DETECTED} detected, {MISSED} missed, {ERROR} errors".format(**counts)
    )
    return 0 if ok else 1


def main(argv):
    commands = {"prove": prove, "mutations": mutations}
    if len(argv) != 2 or argv[1] not in commands:
        print(USAGE)
        return 2
    return commands[argv[1]]()


if __name__ == "__main__":
    sys.exit(main(sys.argv))
