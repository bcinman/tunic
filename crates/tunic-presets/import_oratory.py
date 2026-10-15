#!/usr/bin/env python3
"""Import the saved Reddit wiki through cached original PDFs (macOS PDFKit)."""

import argparse
import concurrent.futures
import copy
import datetime
import hashlib
import html
import json
from pathlib import Path
import re
import subprocess
import unicodedata
import urllib.parse

ROOT = Path(__file__).resolve().parent


def inventory(path):
    saved = json.loads(path.read_text())
    if "entries" in saved:
        keys = ("brand", "model", "target", "variant", "source_url", "key")
        rows = [{key: row[key] for key in keys} for row in saved["entries"]]
        return dict(revision_id=saved["index_revision"], revision_date=saved["index_revision_date"]), rows
    data = saved["data"]
    rows = []
    brand = model = None
    for line in data["content_md"].splitlines():
        if "|" not in line or "https://" not in line:
            continue
        cells = line.split("|")
        if len(cells) != 4:
            raise ValueError(f"Unexpected index row: {line}")
        b, m, target, variant = (html.unescape(cell.strip()) for cell in cells)
        if b != "-":
            brand = b
        if m != "-":
            model = m
        label, url = re.fullmatch(r"\[(.*)\]\((https://.*)\)", target).groups()
        url = html.unescape(url)
        rows.append(dict(brand=brand, model=model, target=label, variant=variant,
                         source_url=url, key=hashlib.sha256(url.encode()).hexdigest()[:16]))
    return data, rows


def download(row, cache):
    path = cache / (row["key"] + ".pdf")
    if path.exists() and path.read_bytes().startswith(b"%PDF"):
        return None
    url = row["source_url"]
    parts = urllib.parse.urlsplit(url)
    query = dict(urllib.parse.parse_qsl(parts.query))
    query["dl"] = "1"
    url = urllib.parse.urlunsplit(parts._replace(
        netloc="dl.dropboxusercontent.com", query=urllib.parse.urlencode(query)))
    result = subprocess.run(
        ["curl", "-fLsS", "--retry", "2", "--retry-delay", "3", "--max-time", "90",
         url, "-o", str(path)], capture_output=True, text=True)
    if result.returncode or not path.read_bytes().startswith(b"%PDF"):
        return result.stderr.strip() or "Response was not a PDF"
    return None


NUMBER = r"[-+]?\d+(?:[.,]\d+)?"
BAND = re.compile(
    rf"Band\s+(\d+)\s+([A-Z_]+)\s+({NUMBER})\s*Hz\s+({NUMBER})\s*dB\s+({NUMBER})"
    rf"(?:[ \t]+({NUMBER})(?![\d.,]))?")
LEGACY_BAND = re.compile(
    rf"Band\s+(\d+)\s+({NUMBER})\s*Hz\s+({NUMBER})\s+({NUMBER})\s*dB\s+([A-Z_]+)")
KINDS = {"PEAK": "peaking", "LOW_SHELF": "low_shelf", "HIGH_SHELF": "high_shelf"}


def number(value):
    return float(value.replace(",", "."))


def parse_pdf(text, preamp_override=None):
    """Reject incomplete tables; only expose unambiguous gain recommendations."""
    if text.count("Filter Settings") != 1:
        raise ValueError(f"Found {text.count('Filter Settings')} tables; their relationship must be reviewed before import")
    table = text
    legacy = "Frequency Q-Factor Gain Filter Type" in table
    pattern = LEGACY_BAND if legacy else BAND
    matches = list(pattern.finditer(table))
    if not matches:
        raise ValueError("No fully specified filters extracted")
    type_tokens = re.findall(r"\b(?:PEAK|LOW_SHELF|HIGH_SHELF|LOW_PASS|HIGH_PASS)\b", table)
    if len(type_tokens) != len(matches):
        raise ValueError("Filter type count differs from complete rows; possible incomplete table")
    filters = []
    for match in matches:
        if legacy:
            band, frequency, quality, gain, kind = match.groups()
        else:
            band, kind, frequency, gain, quality, _ = match.groups()
        if kind not in KINDS:
            raise ValueError(f"Unsupported filter type {kind} in band {band}")
        filters.append(dict(id=int(band), kind=KINDS[kind], frequency_hz=number(frequency),
                            gain_db=number(gain), quality_factor=number(quality)))
    ids = [f["id"] for f in filters]
    if ids != sorted(set(ids)):
        raise ValueError(f"Unordered or repeated band IDs: {ids}")
    remainder = pattern.sub("", table)
    if re.search(r"Band\s+\d+\s+(?!EQ\b|ID\b)[A-Z_]{2,}\b", remainder):
        raise ValueError("Unparsed populated band in table")
    preamps = re.findall(rf"^[ \t/]*(?:Band\s+\d+\s+)*({NUMBER})\s*dB\s*$", remainder, re.M)
    if preamp_override is not None:
        preamps = [str(preamp_override)]
    if len(preamps) != 1:
        raise ValueError(f"Ambiguous preamp: {preamps}")
    dates = re.findall(r"^(\d{2})\.(\d{2})\.(\d{4}|\d{2})[ \t]*$", text, re.M)
    dates = [f"{year if len(year) == 4 else '20' + year}-{month}-{day}" for day, month, year in dates]
    dates += re.findall(r"^(\d{4}-\d{2}-\d{2})[ \t]*$", text, re.M)
    dates = sorted(set(dates))
    if len(dates) > 1:
        raise ValueError(f"Ambiguous source date: {dates}")
    date = datetime.date.fromisoformat(dates[0]).isoformat() if dates else "undated"
    targets = {t for t in re.findall(r"^(.+ Target)\s*$", text, re.M) if "Deviation" not in t}
    if len(targets) != 1:
        raise ValueError(f"Ambiguous target: {sorted(targets)}")
    target = targets.pop().removesuffix(" Target")
    notes = [line.strip() for line in text.splitlines()
             if re.search(r"adjust|preference \(|do not|don't|optional|deactivat", line, re.I)]
    issues = []
    unconfigured = sorted(set(map(int, re.findall(r"\bf(\d+)\b", text))) - set(ids))
    if unconfigured:
        issues.append(f"Source plot references unconfigured bands {unconfigured}; retained only the explicit table rows")
    if date == "undated":
        issues.append("Source PDF has no extracted date; revision uses undated rather than an invented date")
    if preamp_override is not None:
        issues.append("Preamp missing from extracted text; manually verified against PDF (hash-locked override)")
    controls = re.findall(r"Adjust gain\s*of (?:band|filter)\s*(\d+)\s*to (?:preference|taste)\s*\(([^)\n]+)\)", text, re.I)
    if len(controls) != len(re.findall(r"Adjust", text, re.I)):
        issues.append("Unmapped adjustment wording (possibly typo, multi-band, or non-gain); see full source text in manifest")
    for band, label in controls:
        band = int(band)
        if band not in ids:
            issues.append(f"Omitted control for nonexistent band {band}: {label}")
            continue
        f = next(f for f in filters if f["id"] == band)
        label = re.sub(r"\s*/\s*", " / ", label.strip())
        label = label[0].upper() + label[1:]
        freq = f["frequency_hz"]
        suspect = ((label.lower() == "bass" and freq > 500)
                   or (label.lower() in ("warmth", "warmth / muddiness") and freq > 1000)
                   or (label.lower() == "airiness" and freq < 4000))
        if suspect:
            issues.append(f"Omitted suspicious control '{label}' on band {band} ({freq:g} Hz)")
        elif "control_name" in f:
            issues.append(f"Multiple recommendations for band {band}; omitted ambiguous control")
            f["control_name"] = None
        else:
            f["control_name"] = label
    for f in filters:
        if f.get("control_name", "") is None:
            del f["control_name"]
    names = [f.get("control_name") for f in filters]
    for f in filters:
        label = f.get("control_name")
        if label and names.count(label) > 1:
            f["control_name"] = f"{label} (band {f['id']})"
            issues.append(f"Disambiguated repeated control label '{label}' with band number")
    for f in filters:
        if f["frequency_hz"] <= 0 or f["quality_factor"] <= 0:
            raise ValueError(f"Invalid band {f['id']}: frequency {f['frequency_hz']:g} Hz, Q {f['quality_factor']:g}; no guessed correction applied")
        if f["frequency_hz"] >= 22050:
            raise ValueError(f"Band {f['id']} reaches Nyquist at 44.1 kHz")
    return dict(preamp_gain_db=number(preamps[0]), filters=filters), date, target, notes, issues


def slug(value):
    value = unicodedata.normalize("NFKD", value).encode("ascii", "ignore").decode().lower()
    return re.sub(r"[^a-z0-9]+", "-", value).strip("-")


def generate(data, rows, cache):
    overrides = json.loads((ROOT / "oratory1990-overrides.json").read_text())
    existing = {}
    for path in (ROOT / "data/oratory1990").rglob("*.json"):
        doc = json.loads(path.read_text())
        existing[(html.unescape(doc["summary"]["brand"]), html.unescape(doc["summary"]["model"]),
                  doc["attribution"]["source_url"])] = (path, doc)
    records = []
    for row in rows:
        record = dict(row)
        pdf, txt = (cache / (row["key"] + suffix) for suffix in (".pdf", ".txt"))
        try:
            if not pdf.exists() or not pdf.read_bytes().startswith(b"%PDF") or not txt.exists():
                raise ValueError("PDF download or text extraction unavailable")
            record["sha256"] = hashlib.sha256(pdf.read_bytes()).hexdigest()
            record["source_text"] = txt.read_text()
            override = overrides.get(row["key"], {})
            if override and override["sha256"] != record["sha256"]:
                raise ValueError("PDF changed since manual override; re-review required")
            eq, date, target, notes, issues = parse_pdf(record["source_text"], override.get("preamp_gain_db"))
            record.update(source_date=date, recommendations=notes, issues=issues)
            prior = existing.get((row["brand"], row["model"], row["source_url"]))
            if prior:
                path, doc = prior
                before = copy.deepcopy(doc)
                old_date, old_revision = doc["summary"]["revision"].rsplit(".", 1)
                doc["equalizer"] = eq
                doc["summary"]["target"] = target
                doc["summary"]["brand"] = row["brand"]
                doc["summary"]["model"] = row["model"]
                if row["variant"] not in ("", "Standard"):
                    doc["summary"]["variant"] = row["variant"]
                else:
                    doc["summary"].pop("variant", None)
                if before != doc or old_date != date:
                    revision = int(old_revision) + 1 if old_date == date else 1
                    doc["summary"]["revision"] = f"{date}.{revision}"
            else:
                # URL suffix keeps similarly named variants distinct and survives row reordering.
                name = slug(row["model"] + " " + row["variant"] + " " + row["target"])
                name += "-" + row["key"]
                path = ROOT / "data/oratory1990" / slug(row["brand"]) / (name + ".json")
                summary = dict(id=f"oratory1990/{slug(row['brand'])}/{name}", revision=date + ".1",
                               brand=row["brand"], model=row["model"], target=target)
                if row["variant"] not in ("", "Standard"):
                    summary["variant"] = row["variant"]
                doc = dict(summary=summary,
                           attribution=dict(provider="oratory1990", measurement_source="oratory1990",
                                            source_url=row["source_url"]), equalizer=eq)
            path.parent.mkdir(parents=True, exist_ok=True)
            rendered = json.dumps(doc, ensure_ascii=False, indent=2) + "\n"
            # Preserve hand-formatted reviewed files when their data is unchanged.
            if not path.exists() or json.loads(path.read_text()) != doc:
                path.write_text(rendered)
            record.update(status="imported", path=str(path.relative_to(ROOT)), preset_id=doc["summary"]["id"])
        except ValueError as error:
            record.update(status="blocked", issues=[str(error)])
        records.append(record)
    manifest = dict(index_revision=data["revision_id"], index_revision_date=data["revision_date"], entries=records)
    (ROOT / "oratory1990-manifest.json").write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n")
    imported = sum(r["status"] == "imported" for r in records)
    lines = ["# Oratory1990 import issues", "",
             f"{imported}/{len(records)} index entries imported. Each entry is accounted for in `oratory1990-manifest.json`.", "",
             "Generated by `import_oratory.py`; fix extraction or source data before regenerating.",
             "Unsupported or ambiguous tables are blocked, never approximated. Questionable controls are omitted.", "",
             "## Catalog UI follow-up", "",
             "- The native profile picker displays only brand/model, so variants and targets have identical labels.",
             "  Its FFI preset summary includes variant but not target. Expose target and show both qualifiers.",
             "- GPUI displays target but not variant. Include configuration labels there too.",
             "- The native picker is a flat menu of over 1,000 entries; add search/grouping in a separate UI change.", ""]
    for r in records:
        if r["issues"]:
            lines += [f"## {r['brand']} {r['model']} — {r['variant'] or 'Standard'} — {r['target']}", "",
                      f"Status: **{r['status']}**. [Original PDF]({r['source_url']})", ""]
            if "path" in r:
                lines += [f"Preset: `{r['path']}`", ""]
            lines += [f"- {issue}" for issue in r["issues"]]
            lines += [""]
    (ROOT / "ISSUES.md").write_text("\n".join(lines))
    print(f"Imported {imported}/{len(records)}; {sum(bool(r['issues']) for r in records)} entries have issues", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("index", type=Path)
    parser.add_argument("cache", type=Path)
    parser.add_argument("--offline", action="store_true", help="Use already extracted cache only")
    args = parser.parse_args()
    args.cache.mkdir(parents=True, exist_ok=True)
    data, rows = inventory(args.index)
    if args.offline:
        generate(data, rows, args.cache)
        return
    unique = {row["key"]: row for row in rows}
    failures = {}
    with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
        tasks = {pool.submit(download, row, args.cache): row for row in unique.values()}
        for count, task in enumerate(concurrent.futures.as_completed(tasks), 1):
            row = tasks[task]
            error = task.result()
            if error:
                failures[row["key"]] = error
            if count % 25 == 0 or error:
                print(f"Downloaded {count}/{len(unique)}; failures {len(failures)}", flush=True)
    (args.cache / "download-errors.json").write_text(json.dumps(failures, indent=2) + "\n")
    subprocess.run(["swift", str(ROOT / "extract_pdf.swift"), str(args.cache)], check=True)
    generate(data, rows, args.cache)


if __name__ == "__main__":
    main()
