#!/usr/bin/env python3
"""A stand-in for the shipped `virgil repos` command and the proposed
`virgil rulebook` verbs, for the record store tests. It speaks the JSON
lines and exit codes of Virgil's crates/virgil-cli/src/handlers/repos.rs,
and for the rulebook the contract src/store/virgil.rs pins
(virgil.rulebook/v1, one JSON request on --input -, one JSON line out,
every request naming its "scope": "repository" or "private"), keeps its
state in FAKE_VIRGIL_STATE,
and logs every call with the VIRGIL_VIA it saw. The rulebook's records live
in state["rulebook"][scope] and its batch markers in
state["rulebook_markers"][scope].

Knobs (in the state file's "knobs"): "attached" ([group, repository] or
null), "fail" ({op: [exit, landed]}) for the next call of an op (rulebook
ops are named "rulebook.<op>"), "page" (list page size), "rulebook_api"
(the api_version the rulebook answers with), "no_rulebook" (rulebook is an
unknown family, as on a Virgil without the collection).
"""

import json
import os
import re
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
state.setdefault("n", 0)
state.setdefault("rulebook", {"repository": [], "private": []})
state.setdefault("rulebook_markers", {"repository": {}, "private": {}})
knobs = state.setdefault("knobs", {"attached": None, "fail": {}, "page": 100})
with open(log_path, "a") as log:
    log.write(
        json.dumps(
            {"argv": args, "via": os.environ.get("VIRGIL_VIA"), "cwd": os.getcwd()}
        )
        + "\n"
    )

CLIENT = re.compile(r"^[a-z0-9][a-z0-9._-]{0,63}$")


def save():
    json.dump(state, open(state_path, "w"))


def out(api, payload, code=0):
    save()
    print(json.dumps(dict({"api_version": api}, **payload)))
    sys.exit(code)


def error(api, code, message, exit_code):
    out(api, {"error": {"code": code, "message": message, "hint": "fake"}}, exit_code)


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
        out(
            api, {"result": {"marker": request["marker"]}}, failing[0] if failing else 0
        )

    if op == "markers":
        out(
            api,
            {
                "result": {
                    "markers": [
                        {"marker": k, "members": v} for k, v in sorted(markers.items())
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

error("virgil." + family + "/v1", "invalid_request", "unknown " + " ".join(args[:2]), 2)
