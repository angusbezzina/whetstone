#!/usr/bin/env python3
"""A stand-in for the shipped `virgil` CLI (lessons, tasks, repos) and the
proposed `virgil rulebook` verbs, for tests/lessons_virgil.rs and the
record store tests. It speaks the flags, JSON lines and exit codes of
Virgil's crates/virgil-cli/src/handlers/{lessons,tasks,repos}.rs, and for
the rulebook the contract src/store/virgil.rs pins (virgil.rulebook/v1, one
JSON request on --input -, one JSON line out, every request naming its
"scope": "repository" or "private"), keeps its state in FAKE_VIRGIL_STATE,
and logs every call with the VIRGIL_VIA it saw. The rulebook's records live
in state["rulebook"][scope] and its batch markers in
state["rulebook_markers"][scope].

Knobs (in the state file's "knobs"): "attached" ([group, repository] or
null), "fail" ({op: [exit, landed]}) for the next call of an op (rulebook
ops are named "rulebook.<op>"), "stale_get" (get answers one revision
behind), "page" (list page size), "rulebook_api" (the api_version the
rulebook answers with), "no_rulebook" (rulebook is an unknown family, as on
a Virgil without the collection).
"""

import hashlib
import json
import os
import re
import subprocess
import sys

args = sys.argv[1:]
# Without FAKE_VIRGIL_STATE (and FAKE_VIRGIL_LOG) the state lives in the
# checkout the call runs in, so parallel tests never share a fake.
local = os.path.join(os.getcwd(), ".fake-virgil")
state_path = os.environ.get("FAKE_VIRGIL_STATE") or os.path.join(local, "state.json")
log_path = os.environ.get("FAKE_VIRGIL_LOG") or os.path.join(local, "calls.jsonl")
os.makedirs(os.path.dirname(state_path), exist_ok=True)
os.makedirs(os.path.dirname(log_path), exist_ok=True)
state = json.load(open(state_path)) if os.path.exists(state_path) else {}
state.setdefault("lessons", {})
state.setdefault("tasks", {})
state.setdefault("seen", {})
state.setdefault("n", 0)
state.setdefault("rulebook", {"repository": [], "private": []})
state.setdefault("rulebook_markers", {"repository": {}, "private": {}})
knobs = state.setdefault(
    "knobs", {"attached": None, "fail": {}, "stale_get": False, "page": 100}
)
with open(log_path, "a") as log:
    log.write(
        json.dumps(
            {"argv": args, "via": os.environ.get("VIRGIL_VIA"), "cwd": os.getcwd()}
        )
        + "\n"
    )

CROCKFORD = re.compile(r"^[0-7][0-9A-HJKMNP-TV-Z]{25}$")
CLIENT = re.compile(r"^[a-z0-9][a-z0-9._-]{0,63}$")


def save():
    json.dump(state, open(state_path, "w"))


def out(api, payload, code=0):
    save()
    print(json.dumps(dict({"api_version": api}, **payload)))
    sys.exit(code)


def error(api, code, message, exit_code):
    out(api, {"error": {"code": code, "message": message, "hint": "fake"}}, exit_code)


def flags(rest):
    """--flag=value pairs (repeatable) and positionals; a bare --flag is True."""
    named, positional = {}, []
    for arg in rest:
        if arg.startswith("--"):
            name, _, value = arg[2:].partition("=")
            named.setdefault(name, []).append(value if "=" in arg else True)
        else:
            positional.append(arg)
    return named, positional


def one(named, name, default=None):
    return named.get(name, [default])[-1]


def head_blob(path):
    return (
        subprocess.run(
            ["git", "cat-file", "-t", "HEAD:" + path], capture_output=True
        ).stdout
        == b"blob\n"
    )


def sha(path):
    try:
        return hashlib.sha256(open(path, "rb").read()).hexdigest()
    except OSError:
        return ""


def view(lesson):
    shown = dict(lesson)
    about = []
    for subject in lesson["about"]:
        path = subject["subject"].get("Path")
        if path:
            path = dict(path, stale=sha(path["path"]) != path["sha256"])
            subject = {"subject": {"Path": path}}
        about.append(subject)
    shown["about"] = about
    return shown


def stale(lesson):
    return any(
        s["subject"]["Path"]["stale"]
        for s in view(lesson)["about"]
        if "Path" in s["subject"]
    )


family = args[0]
if family == "repos":
    attached = knobs["attached"]
    if attached is None:
        out("virgil.repos/v1", {"result": {"resolution": None, "suggestions": []}})
    group, repository = attached
    out(
        "virgil.repos/v1",
        {
            "result": {
                "resolution": {
                    "repository": {
                        "identity": {"group": group, "repository": repository},
                        "name": "fixture",
                    },
                    "root": os.getcwd(),
                }
            }
        },
    )

if family == "lessons":
    api = "virgil.lessons/v1"
    op = args[1]
    named, positional = flags(args[2:])
    if knobs["attached"] is None:
        error(api, "unattached", "use virgil repos attach to select a repository", 6)
    group, repository = knobs["attached"]
    lessons = state["lessons"]
    failing = knobs["fail"].pop(op, None)

    if op == "import-beads":
        source = os.path.realpath(one(named, "file"))
        if source.startswith(os.path.realpath(os.getcwd()) + os.sep):
            error(
                "virgil.lessons.import-beads/v1",
                "invalid_request",
                "export is inside the repository",
                2,
            )
        exported = json.load(open(source))
        entries = []
        for key, body in sorted(exported.items()):
            header, _, text = body.partition("\n")
            parts = header.split(" ")
            status = parts[0]
            scope = parts[1].split("=", 1)[1]
            disposition = (
                "excluded"
                if status == "OBSOLETE"
                else ("personal_needs_owner" if scope == "personal" else "import")
            )
            entries.append({"key": key, "disposition": disposition})
            if named.get("apply") and status != "OBSOLETE":
                state["n"] += 1
                lessons[key] = {
                    "id": "01J9%022d" % state["n"],
                    "key": key,
                    "text": text,
                    "revision": 1,
                    "status": status.lower(),
                    "trust": "validated",
                    "via": "did:key:person",
                    "writer_kind": "person",
                    "created_by": "principal:OWNER",
                    "collection": "personal_lesson"
                    if scope == "personal"
                    else "lesson",
                    "about": [{"subject": {"Repository": "repository:" + repository}}]
                    if scope == "repo"
                    else [],
                    "successor": (parts[4].split("=", 1)[1] if len(parts) > 4 else ""),
                }
        out("virgil.lessons.import-beads/v1", {"result": {"entries": entries}})

    if op in ("get", "list", "find"):
        if op == "get":
            lesson = lessons.get(positional[0])
            if lesson is None:
                error(api, "not_found", "lesson is missing at the confirmed cut", 6)
            shown = view(lesson)
            if knobs["stale_get"]:
                shown["revision"] -= 1
            out(api, {"result": {"lessons": [shown], "cursor": "", "truncated": False}})
        found = [view(lesson) for _, lesson in sorted(lessons.items())]
        if one(named, "changed"):
            found = [lesson for lesson in found if stale(lessons[lesson["key"]])]
        start = int(one(named, "cursor", "0") or "0")
        size = min(int(one(named, "limit", "10")), knobs["page"])
        page = found[start : start + size]
        cursor = str(start + size) if start + size < len(found) else ""
        out(api, {"result": {"lessons": page, "cursor": cursor, "truncated": False}})

    # Writes: a ULID idempotency key, a well-formed marker, replayable.
    key = one(named, "idempotency-key", "")
    if not CROCKFORD.match(key):
        error(api, "invalid_request", "idempotency key must be a ULID", 2)
    via = os.environ.get("VIRGIL_VIA")
    if via is not None and not (via.startswith("agent:") and CLIENT.match(via[6:])):
        error(
            api,
            "invalid_request",
            "expected agent:<client> with a lowercase client name",
            2,
        )
    trust = "inferred" if via else "validated"
    shown_via = via or "did:key:person"
    if key in state["seen"]:
        out(api, {"idempotency_key": key, "result": state["seen"][key]})

    def mutation(lesson, code=0):
        result = {
            "id": lesson["id"],
            "revision": lesson["revision"],
            "via": shown_via,
            "trust": trust,
            "replication_pending": code == 7,
            "rejection": {"why": "follow-up refused"} if code in (3, 4, 5) else None,
        }
        state["seen"][key] = result
        out(api, {"idempotency_key": key, "result": result}, code)

    if failing and not failing[1]:
        codes = {
            3: "read_again",
            4: "refused_content",
            5: "not_permitted",
            6: "not_found",
            7: "unavailable",
        }
        error(api, codes.get(failing[0], "internal"), "fake refusal", failing[0])

    if op == "remember":
        lesson_key, text = one(named, "key"), one(named, "text")
        revision = one(named, "if-revision")
        if revision is not None and not one(named, "reason"):
            error(api, "invalid_request", "--if-revision requires --reason", 2)
        about = []
        for subject in named.get("about", []):
            if subject == "repo":
                about.append({"subject": {"Repository": "repository:" + repository}})
            elif subject.startswith("record:"):
                about.append({"subject": {"Locator": subject[7:]}})
            else:
                error(
                    api,
                    "invalid_request",
                    "about must be repo, record:<locator> or path:<path>",
                    2,
                )
        for anchor in named.get("anchor", []):
            path, _, lines = anchor.partition("#L")
            if not head_blob(path):
                error(
                    api,
                    "invalid_request",
                    "anchor is not a file at the resolved commit",
                    2,
                )
            subject = {
                "repository": "repository:" + repository,
                "path": path,
                "commit": "HEAD",
                "sha256": sha(path),
            }
            if lines:
                start, _, end = lines.partition("-L")
                subject.update(line_start=int(start), line_end=int(end or start))
            about.append({"subject": {"Path": subject}})
        current = lessons.get(lesson_key)
        if revision is None and current is not None:
            error(
                api,
                "invalid_request",
                "lesson key exists; pass --if-revision to rewrite it",
                2,
            )
        if revision is not None:
            if current is None:
                error(api, "not_found", "lesson missing at confirmed cut", 6)
            if int(revision) != current["revision"]:
                error(
                    api,
                    "read_again",
                    "revision rejected; current revision is %d" % current["revision"],
                    3,
                )
            current.update(
                text=text,
                about=about,
                revision=current["revision"] + 1,
                trust=trust,
                via=shown_via,
                writer_kind="agent" if via else "person",
            )
            mutation(current, failing[0] if failing else 0)
        audience = one(named, "audience", "channel")
        lesson = {
            "id": key,
            "key": lesson_key,
            "text": text,
            "revision": 1,
            "status": "current",
            "trust": trust,
            "via": shown_via,
            "writer_kind": "agent" if via else "person",
            "created_by": "principal:P",
            "collection": "personal_lesson" if audience == "personal" else "lesson",
            "about": about,
            "successor": "",
        }
        lessons[lesson_key] = lesson
        mutation(lesson, failing[0] if failing else 0)

    old = lessons.get(positional[0]) if positional else None
    if old is None:
        error(api, "not_found", "lesson missing at confirmed cut", 6)
    if int(one(named, "if-revision", "-1")) != old["revision"]:
        error(
            api,
            "read_again",
            "revision rejected; current revision is %d" % old["revision"],
            3,
        )
    if op == "supersede":
        successor = lessons.get(one(named, "with"))
        if successor is None:
            error(api, "not_found", "successor missing at confirmed cut", 6)
        if int(one(named, "successor-revision", "-1")) != successor["revision"]:
            error(api, "read_again", "successor revision rejected", 3)
        old.update(
            status="superseded", successor=successor["id"], revision=old["revision"] + 1
        )
        mutation(old)
    if op == "forget":
        old.update(status="forgotten", revision=old["revision"] + 1)
        mutation(old)
    error(api, "invalid_request", "unknown op " + op, 2)

if family == "--version":
    print("virgil 0.0.0-fake")
    sys.exit(0)

if family == "rulebook" and knobs.get("no_rulebook"):
    # A Virgil without the collection: clap refuses the subcommand.
    save()
    sys.stderr.write("error: unrecognized subcommand 'rulebook'\n")
    sys.exit(2)

if family == "rulebook":
    api = knobs.get("rulebook_api", "virgil.rulebook/v1")
    op = args[1]
    if args[2:] != ["--input", "-"]:
        error(api, "invalid_request", "rulebook verbs read --input -", 2)
    request = json.loads(sys.stdin.read() or "{}")
    scope = request.get("scope")
    if scope not in ("repository", "private"):
        error(api, "invalid_request", "scope must be repository or private", 2)
    if op == "import-beads" and scope != "repository":
        error(api, "invalid_request", "import-beads writes the repository scope", 2)
    if knobs["attached"] is None:
        error(api, "unattached", "use virgil repos attach to select a repository", 6)
    group, repository = knobs["attached"]
    records = state["rulebook"][scope]
    markers = state["rulebook_markers"][scope]
    failing = knobs["fail"].pop("rulebook." + op, None)
    codes = {
        3: "read_again",
        4: "refused_content",
        5: "not_permitted",
        6: "not_found",
        7: "unavailable",
    }
    if failing and not failing[1]:
        error(api, codes.get(failing[0], "internal"), "fake refusal", failing[0])
    via = os.environ.get("VIRGIL_VIA")
    if via is not None and not (via.startswith("agent:") and CLIENT.match(via[6:])):
        error(api, "invalid_request", "expected agent:<client>", 2)

    def find(record_id, revision=None):
        found = [a for a in records if a["record_id"] == record_id]
        if revision is not None:
            found = [a for a in found if a["revision"] == revision]
        return sorted(found, key=lambda a: a["revision"])

    def store(agreement):
        state["n"] += 1
        stored = dict(agreement, id="01JA%022d" % state["n"])
        stored.pop("evidence", None)
        records.append(stored)
        return stored

    def landed(stored, replayed):
        code = failing[0] if failing else 0
        out(
            api,
            {
                "result": {
                    "id": stored["id"],
                    "record_id": stored["record_id"],
                    "revision": stored["revision"],
                    "digest": stored["digest"],
                    "replayed": replayed,
                    "replication_pending": code == 7,
                }
            },
            code,
        )

    if op == "put":
        agreement = request.get("record") or {}
        for field in (
            "record_id",
            "revision",
            "digest",
            "idempotency_key",
            "wh_record",
        ):
            if field not in agreement:
                error(api, "invalid_request", "record needs " + field, 2)
        key = agreement["idempotency_key"]
        same_key = [a for a in records if a["idempotency_key"] == key]
        if same_key:
            if (
                same_key[0]["digest"] == agreement["digest"]
                and same_key[0]["record_id"] == agreement["record_id"]
                and same_key[0]["revision"] == agreement["revision"]
            ):
                landed(same_key[0], True)
            error(api, "idempotency_conflict", "the key recorded other content", 4)
        if not request.get("copy"):
            history = find(agreement["record_id"])
            actual = history[-1]["revision"] if history else None
            if request.get("expected_revision") != actual:
                out(
                    api,
                    {
                        "error": {
                            "code": "stale_revision",
                            "message": "expected revision rejected",
                            "hint": "fake",
                            "actual": actual,
                        }
                    },
                    3,
                )
            if agreement["revision"] != (actual or 0) + 1:
                error(api, "invalid_request", "revision must follow the latest", 2)
        slot = find(agreement["record_id"], agreement["revision"])
        if slot:
            if slot[0]["digest"] == agreement["digest"]:
                landed(slot[0], True)
            error(api, "conflicting_revision", "that revision holds other content", 4)
        landed(store(agreement), False)

    if op == "get":
        found = find(request.get("record_id"), request.get("revision"))
        if not found:
            error(api, "not_found", "record is missing at the confirmed cut", 6)
        out(api, {"result": {"record": found[0]}})

    if op == "latest":
        found = find(request.get("record_id"))
        if not found:
            error(api, "not_found", "no record with that record id", 6)
        out(api, {"result": {"record": found[-1]}})

    if op == "history":
        out(api, {"result": {"records": find(request.get("record_id"))}})

    if op == "list":
        found = sorted(records, key=lambda a: (a["record_id"], a["revision"]))
        if request.get("idempotency_key"):
            found = [
                a for a in found if a["idempotency_key"] == request["idempotency_key"]
            ]
        start = int(request.get("cursor") or "0")
        size = min(int(request.get("limit", 100)), knobs["page"])
        page = found[start : start + size]
        cursor = str(start + size) if start + size < len(found) else ""
        out(api, {"result": {"records": page, "cursor": cursor}})

    if op == "mark":
        markers[request["marker"]] = request.get("members", [])
        out(api, {"result": {"marker": request["marker"]}}, failing[0] if failing else 0)

    if op == "markers":
        out(
            api,
            {
                "result": {
                    "markers": [
                        {"marker": k, "members": v}
                        for k, v in sorted(markers.items())
                    ]
                }
            },
        )

    if op == "lifecycle":
        found = find(request.get("record_id"), request.get("revision"))
        if not found:
            error(api, "not_found", "record is missing at the confirmed cut", 6)
        found[0]["lifecycle"] = request["lifecycle"]
        out(
            api,
            {
                "result": {
                    "record_id": found[0]["record_id"],
                    "revision": found[0]["revision"],
                    "lifecycle": found[0]["lifecycle"],
                }
            },
            failing[0] if failing else 0,
        )

    if op == "import-beads":
        source = os.path.realpath(request.get("file", ""))
        if source.startswith(os.path.realpath(os.getcwd()) + os.sep):
            error(api, "invalid_request", "export is inside the repository", 2)
        exported = json.load(open(source))
        if exported.get("schema") != "whetstone.rulebook-export/v1":
            error(api, "invalid_request", "not a Whetstone rulebook export", 2)
        entries = []
        for agreement in exported["records"]:
            slot = find(agreement["record_id"], agreement["revision"])
            if slot:
                disposition = (
                    "present"
                    if slot[0]["digest"] == agreement["digest"]
                    else "conflict"
                )
            else:
                disposition = "import"
                if request.get("apply"):
                    store(agreement)
            entries.append(
                {
                    "record_id": agreement["record_id"],
                    "revision": agreement["revision"],
                    "digest": agreement["digest"],
                    "disposition": disposition,
                }
            )
        out(
            api, {"result": {"entries": entries, "applied": bool(request.get("apply"))}}
        )

    error(api, "invalid_request", "unknown op " + op, 2)

if family == "tasks":
    api = "virgil.tasks/v1"
    op = args[1]
    request = json.loads(sys.stdin.read() or "{}")
    tasks = state["tasks"]
    if op == "create":
        ref = "task:01J8Z0000000000000000000T%d" % (len(tasks) + 1)
        tasks[ref] = {
            "task": ref,
            "title": request["title"],
            "description": request["description"],
            "state": "todo",
            "comments": [],
        }
        out(
            api,
            {"result": {"scope": request.get("scope"), "task": ref, "queued": False}},
        )
    ref = args[2]
    task = tasks[ref]
    if op == "show":
        detail = {k: v for k, v in task.items() if k != "comments"}
        out(
            api,
            {
                "result": {
                    "task": detail,
                    "files": [],
                    "comments": [{"message": "m", "text": c} for c in task["comments"]],
                    "links": {},
                }
            },
        )
    if op == "comment":
        task["comments"].append(request["text"])
        out(api, {"result": {"task": ref, "message": "m", "queued": False}})
    if op == "move":
        task["state"] = request["state"]
        out(
            api,
            {
                "result": {
                    "task": ref,
                    "state": task["state"],
                    "changed": True,
                    "queued": False,
                }
            },
        )
error("virgil." + family + "/v1", "invalid_request", "unknown " + " ".join(args[:2]), 2)
