#!/usr/bin/env python3
"""A stand-in for the shipped `virgil` CLI (lessons, tasks, repos), for
tests/lessons_virgil.rs. It speaks the flags, JSON lines and exit codes of
Virgil's crates/virgil-cli/src/handlers/{lessons,tasks,repos}.rs, keeps its
state in FAKE_VIRGIL_STATE, and logs every call with the VIRGIL_VIA it saw.

Knobs (in the state file's "knobs"): "attached" ([group, repository] or
null), "fail" ({op: [exit, landed]}) for the next call of an op, "stale_get"
(get answers one revision behind), "page" (list page size).
"""

import hashlib
import json
import os
import re
import subprocess
import sys

args = sys.argv[1:]
state_path = os.environ["FAKE_VIRGIL_STATE"]
state = json.load(open(state_path)) if os.path.exists(state_path) else {}
state.setdefault("lessons", {})
state.setdefault("tasks", {})
state.setdefault("seen", {})
state.setdefault("n", 0)
knobs = state.setdefault(
    "knobs", {"attached": None, "fail": {}, "stale_get": False, "page": 100}
)
with open(os.environ["FAKE_VIRGIL_LOG"], "a") as log:
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
