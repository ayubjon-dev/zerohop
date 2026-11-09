"""Attack simulation for the revised Zerohop ledger rules. Signatures are assumed valid."""
import itertools
import random
from collections import defaultdict
from math import inf


def add(author, seq, device):
    return ("add", author, seq, device, None)


def rev(author, seq, device, last_seen):
    return ("rev", author, seq, device, last_seen)


def gen(author):
    return ("gen", author, 1, None, None)


def membership(events, genesis="G", max_adds=32, max_live=64):
    events = set(events)
    by = defaultdict(lambda: defaultdict(set))
    for e in events:
        by[e[1]][e[2]].add(e)

    # Highest contiguous seq per author, and the equivocation cut.
    top, eq_cut = {}, {}
    for a, seqs in by.items():
        h = 0
        while h + 1 in seqs:
            h += 1
        top[a] = h
        forks = [s for s, v in seqs.items() if len(v) > 1]
        if forks:
            eq_cut[a] = min(forks) - 1

    usable = lambda e: e[2] <= top[e[1]]

    def adds_of(author):
        rows = sorted(
            (e for e in events if e[0] == "add" and e[1] == author and usable(e)),
            key=lambda e: (e[2], str(e[3])),
        )
        allowed = set(sorted({e[2] for e in rows})[:max_adds])
        return [e for e in rows if e[2] in allowed]

    # Step: ever-members. No cutoffs, forks included, so authority only grows.
    ever = {genesis}
    changed = True
    while changed:
        changed = False
        for a in list(ever):
            for e in adds_of(a):
                if e[3] not in ever:
                    ever.add(e[3])
                    changed = True

    # Step: every Revoke from an ever-member counts, permanently.
    revokes = [
        e for e in events if e[0] == "rev" and usable(e) and e[1] in ever and e[3] in ever
    ]
    revoked = {e[3] for e in revokes} | (set(eq_cut) & ever)

    cutoff = defaultdict(lambda: inf)
    for a, c in eq_cut.items():
        cutoff[a] = min(cutoff[a], c)
    for e in revokes:
        value = e[2] - 1 if e[1] == e[3] else e[4]
        cutoff[e[3]] = min(cutoff[e[3]], value)

    # Step: live closure, respecting cutoffs, breadth first in a fixed order.
    live, frontier = {genesis}, [genesis]
    while frontier:
        nxt = []
        for a in sorted(frontier):
            for e in adds_of(a):
                room = e[3] in revoked or len(live - revoked) < max_live
                if e[2] <= cutoff[a] and e[3] not in live and room:
                    live.add(e[3])
                    nxt.append(e[3])
        frontier = nxt
    return live - revoked, revoked


def show(name, events, expect):
    got, rev_set = membership(events)
    ok = got == expect
    print(f"{'PASS' if ok else 'FAIL'}  {name}: members={sorted(got)} revoked={sorted(rev_set)}")
    return ok


base = [gen("G"), add("G", 2, "M"), add("G", 3, "P"), add("P", 1, "Y")]
results = []

results.append(show("baseline", base, {"G", "M", "P", "Y"}))

# Reviewer attack 1: a revoked device tries to reinstate itself.
e = base + [rev("Y", 1, "M", 0), rev("M", 1, "P", 0)]
got, _ = membership(e)
results.append(show("revoked M cannot return after its late revoke", e, {"G"}))
assert "M" not in got

# Reviewer attack 2: an outsider's revoke must be ignored.
results.append(show("outsider revoke ignored", base + [rev("X", 1, "G", 0)], {"G", "M", "P", "Y"}))

# A revoked device cannot add a rogue device afterwards.
e = base + [rev("G", 4, "M", 0), add("M", 1, "ROGUE")]
results.append(show("late add after revoke ignored", e, {"G", "P", "Y"}))

# An add made before the revoke and seen by the revoker survives.
e = base + [add("M", 1, "T"), rev("G", 4, "M", 1)]
results.append(show("seen add survives revoke", e, {"G", "P", "Y", "T"}))

# An add the revoker had not seen does not survive.
e = base + [add("M", 1, "T"), rev("G", 4, "M", 0)]
results.append(show("unseen add dropped by revoke", e, {"G", "P", "Y"}))

# Equivocation removes the author and voids the forked events.
e = base + [add("M", 1, "R1"), add("M", 1, "R2")]
results.append(show("equivocator removed, forks void", e, {"G", "P", "Y"}))

# Equivocating must not undo a revoke made by a device the equivocator added.
e = [
    gen("G"),
    add("G", 2, "M"),
    add("G", 3, "Z"),
    add("M", 1, "Y"),
    rev("Y", 1, "Z", 0),
    add("M", 1, "FORK"),
]
got, r = membership(e)
results.append(show("equivocation cannot reinstate Z", e, {"G"}))
assert "Z" in r

# Mutual revoke removes both.
e = [gen("G"), add("G", 2, "A"), add("G", 3, "B"), rev("A", 1, "B", 0), rev("B", 1, "A", 0)]
results.append(show("mutual revoke removes both", e, {"G"}))

# Revoking the genesis device leaves the devices it added.
e = [gen("G"), add("G", 2, "A"), add("G", 3, "B"), rev("A", 1, "G", 3)]
results.append(show("genesis can be revoked", e, {"A", "B"}))

# A gap makes later events unusable until it is filled.
e = base + [add("M", 2, "T")]
results.append(show("gap ignored", e, {"G", "M", "P", "Y"}))
results.append(show("gap filled", e + [add("M", 1, "S")], {"G", "M", "P", "Y", "S", "T"}))

# Per-author add cap is deterministic and does not block a revoke.
flood = [gen("G"), add("G", 2, "M")] + [add("M", i + 1, f"J{i:04}") for i in range(4096)]
e = flood + [rev("G", 3, "M", 0)]
results.append(show("flood then revoke removes all junk", e, {"G"}))

# Second-round findings.
big = 0xFFFFFFFF
e = base + [rev("M", 1, "M", big), add("M", 2, "R")]
results.append(show("inflated self-revoke cannot keep adding", e, {"G", "P", "Y"}))

e = base + [add("M", 1, "X"), rev("X", 1, "M", big), add("M", 2, "R")]
results.append(show("colluding inflated revoke leaves R visible", e, {"G", "P", "Y", "X", "R"}))
results.append(show("revoking again removes R and X", e + [rev("G", 4, "M", 0)], {"G", "P", "Y"}))

adds = [add("A", i + 1, f"D{i:02}") for i in range(31)] + [add("A", 32, "Y")]
e = [gen("G"), add("G", 2, "M"), add("G", 3, "A")] + adds + [rev("Y", 1, "M", 0)]
before, _ = membership(e)
e2 = e + [add("A", 1, "FORK")]
after, r = membership(e2)
ok = "M" not in before and "M" not in after and "M" in r
print(f"{'PASS' if ok else 'FAIL'}  fork pushing out 32nd add cannot reinstate M")
results.append(ok)

ev = [gen("G"), add("G", 2, "A"), rev("G", 3, "A", 0)]
ev += [add("G", 4 + i, f"N{i:02}") for i in range(32)]
got, _ = membership(ev, max_live=10)
ok = len(got) == 10 and "A" not in got
print(f"{'PASS' if ok else 'FAIL'}  member cap ignores revoked devices: {len(got)} members")
results.append(ok)

# Order independence: the result depends only on the set of events.
scenario = base + [rev("Y", 1, "M", 0), rev("M", 1, "P", 0), add("G", 4, "Q"), add("Q", 1, "W")]
answers = set()
for _ in range(2000):
    s = scenario[:]
    random.shuffle(s)
    m, r = membership(s)
    answers.add((tuple(sorted(m)), tuple(sorted(r))))
ok = len(answers) == 1
print(f"{'PASS' if ok else 'FAIL'}  order independence over 2000 shuffles: {answers}")
results.append(ok)

# Monotonic revocation: no subset ordering of added events can take a device out of the revoked set.
core = [gen("G"), add("G", 2, "M"), add("G", 3, "P"), rev("P", 1, "M", 0)]
extras = [
    rev("M", 1, "P", 0),
    add("M", 1, "X"),
    add("M", 1, "X2"),
    rev("M", 2, "G", 0),
    add("P", 2, "K"),
    rev("K", 1, "P", 0),
]
ok = True
for n in range(len(extras) + 1):
    for sub in itertools.combinations(extras, n):
        m, r = membership(core + list(sub))
        if "M" not in r or "M" in m:
            ok = False
            print("  M came back with", sub, "members", m)
print(f"{'PASS' if ok else 'FAIL'}  revoked M never returns across all {2 ** len(extras)} extra-event subsets")
results.append(ok)

print(f"\n{sum(results)}/{len(results)} passed")
