#!/usr/bin/env python3
"""Threshold evaluation for the face attendance system (plan Section 10).

This script is the *only* place a production T_ACCEPT / T_MARGIN should come
from. It reads a labelled score file produced by the pipeline and reports the
ROC, the genuine/impostor score distributions, and the operating threshold that
meets a target FAR.

Input format: CSV with columns
    pair_id,is_genuine,score,margin
one row per compared pair. `is_genuine` is 1 for same-student pairs, 0 for
different-student pairs. `score` is the cosine similarity of the best match.
`margin` is top1 - best_other (may be blank).

Usage:
    python3 eval/thresholds.py eval/data/scores.csv --target-far 1e-5

It does not touch any database or model; it is pure analysis so the numbers can
be reproduced and reviewed.
"""

from __future__ import annotations

import argparse
import csv
import sys
from dataclasses import dataclass


@dataclass
class Pair:
    is_genuine: bool
    score: float
    margin: float | None


def load(path: str) -> list[Pair]:
    pairs: list[Pair] = []
    with open(path, newline="") as fh:
        reader = csv.DictReader(fh)
        required = {"is_genuine", "score"}
        if not required.issubset(reader.fieldnames or []):
            sys.exit(f"missing required columns {required}; got {reader.fieldnames}")
        for row in reader:
            margin_raw = (row.get("margin") or "").strip()
            pairs.append(
                Pair(
                    is_genuine=row["is_genuine"].strip() in {"1", "true", "True"},
                    score=float(row["score"]),
                    margin=float(margin_raw) if margin_raw else None,
                )
            )
    if not pairs:
        sys.exit("no data rows")
    return pairs


def evaluate(pairs: list[Pair], threshold: float) -> tuple[float, float, int, int]:
    """Return (FAR, FRR, false_accepts, false_rejects) at a given threshold.

    Accept when score >= threshold. A false accept is an impostor accepted; a
    false reject is a genuine pair rejected.
    """
    impostors = [p for p in pairs if not p.is_genuine]
    genuines = [p for p in pairs if p.is_genuine]
    false_accepts = sum(1 for p in impostors if p.score >= threshold)
    false_rejects = sum(1 for p in genuines if p.score < threshold)
    far = false_accepts / len(impostors) if impostors else 0.0
    frr = false_rejects / len(genuines) if genuines else 0.0
    return far, frr, false_accepts, false_rejects


def threshold_for_far(pairs: list[Pair], target_far: float) -> float:
    """Lowest threshold whose FAR is <= target (i.e. the most permissive
    threshold that still meets the target FAR)."""
    candidates = sorted({p.score for p in pairs}, reverse=True)
    best = 1.0
    for t in candidates:
        far, _, _, _ = evaluate(pairs, t)
        if far <= target_far:
            best = t  # keep lowering while the target is still met
        else:
            break
    return best


def percentile(values: list[float], q: float) -> float:
    if not values:
        return float("nan")
    values = sorted(values)
    idx = max(0, min(len(values) - 1, int(round(q * (len(values) - 1)))))
    return values[idx]


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("scores", help="CSV of scored pairs")
    ap.add_argument(
        "--target-far",
        type=float,
        default=1e-5,
        help="target false accept rate (default 1e-5, plan Section 6)",
    )
    ap.add_argument(
        "--margin-floor",
        type=float,
        default=0.0,
        help="also require margin >= this (0 = ignore)",
    )
    args = ap.parse_args()

    pairs = load(args.scores)
    impostors = sorted(p.score for p in pairs if not p.is_genuine)
    genuines = sorted(p.score for p in pairs if p.is_genuine)

    print(f"pairs: {len(pairs)}  genuine: {len(genuines)}  impostor: {len(impostors)}")
    if genuines:
        print(
            "genuine score  p1={:.4f} p5={:.4f} p50={:.4f} p95={:.4f}".format(
                percentile(genuines, 0.01),
                percentile(genuines, 0.05),
                percentile(genuines, 0.50),
                percentile(genuines, 0.95),
            )
        )
    if impostors:
        print(
            "impostor score p50={:.4f} p95={:.4f} p99={:.4f} max={:.4f}".format(
                percentile(impostors, 0.50),
                percentile(impostors, 0.95),
                percentile(impostors, 0.99),
                percentile(impostors, 1.0),
            )
        )

    print("\nscore  FAR       FRR")
    for t in [i / 20 for i in range(21)]:
        far, frr, _, _ = evaluate(pairs, t)
        print(f"{t:.2f}   {far:<9.6f} {frr:.6f}")

    rec = threshold_for_far(pairs, args.target_far)
    far, frr, fa, fr = evaluate(pairs, rec)
    print(
        f"\nrecommended T_ACCEPT = {rec:.4f} "
        f"(FAR={far:.2e} [{fa} false accepts], FRR={frr:.4f} [{fr} false rejects])"
    )
    print("Set T_ACCEPT to this value ONLY after reviewing the distributions above.")
    if args.margin_floor > 0:
        print(f"Also require T_MARGIN >= {args.margin_floor}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
