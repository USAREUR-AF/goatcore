#!/usr/bin/env python3
"""Version-aware, bounded FIFO eviction for the GOVERNANCE audit middle tier."""
import argparse
import datetime as dt
import fcntl
import json
import os
from pathlib import Path
import re
import selectors
import subprocess
import tempfile
import time
import uuid


class Failure(RuntimeError):
    code = 3


class Parser(argparse.ArgumentParser):
    def error(self, message):
        raise Failure("invalid command line: " + message)


def atomic_json(path, value):
    fd, name = tempfile.mkstemp(prefix=".evict-", dir=path.parent)
    try:
        with os.fdopen(fd, "w") as stream:
            json.dump(value, stream, sort_keys=True)
            stream.write("\n")
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(name, path)
        directory = os.open(path.parent, os.O_RDONLY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    finally:
        if os.path.exists(name):
            os.unlink(name)


class MC:
    OUTPUT_LIMIT = 128 * 2**20
    TIMEOUT = 180
    def __init__(self, config):
        self.config = config

    def command(self, *args, source=None):
        argv = ["docker", "run", "--rm", "--network", "host", "--env-file",
                self.config["env_file"]]
        if source is not None:
            argv += ["-v", str(source) + ":/tmp/trail.json:ro"]
        argv += ["--entrypoint", "mc", self.config["mc_image"], "--json", *args]
        process = None
        output = bytearray()
        selector = selectors.DefaultSelector()
        try:
            process = subprocess.Popen(argv, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
            selector.register(process.stdout, selectors.EVENT_READ)
            deadline = time.monotonic() + self.TIMEOUT
            while selector.get_map():
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise Failure("mc command timed out")
                events = selector.select(remaining)
                for key, _ in events:
                    chunk = os.read(key.fd, 65536)
                    if not chunk:
                        selector.unregister(key.fileobj)
                        continue
                    output.extend(chunk)
                    if len(output) > self.OUTPUT_LIMIT:
                        raise Failure("mc listing exceeds the bounded output budget; refusing a partial plan")
            process.wait(timeout=max(0.1, deadline - time.monotonic()))
        except (OSError, subprocess.SubprocessError):
            raise Failure("mc command could not complete") from None
        finally:
            selector.close()
            if process is not None:
                if process.poll() is None:
                    process.kill()
                    process.wait()
                process.stdout.close()
        if process.returncode:
            raise Failure("mc " + args[0] + " failed; credentials and provider output withheld")
        try:
            rows = [json.loads(line) for line in output.splitlines() if line.strip()]
        except ValueError:
            raise Failure("mc returned malformed JSON") from None
        if any(not isinstance(row, dict) or row.get("status") != "success" for row in rows):
            raise Failure("mc returned an unsuccessful result")
        return rows

    def used(self, target):
        rows = self.command("du", "--versions", target)
        if len(rows) != 1 or type(rows[0].get("size")) is not int or rows[0]["size"] < 0:
            raise Failure("bucket size is missing or invalid")
        return rows[0]["size"]

    def versions(self, target):
        return self.command("ls", "--recursive", "--versions", target)

    def remove(self, target, version):
        if not self.command("rm", "--bypass", "--version-id", version, target):
            raise Failure("version removal returned no confirmation")

    def trail(self, source, target):
        if not self.command("cp", "/tmp/trail.json", target, source=source):
            raise Failure("eviction trail upload returned no confirmation")


def validate(config):
    for key in ("quota_gib", "high_pct", "low_pct", "batch_hours", "max_pass_pct"):
        if type(config.get(key)) is not int:
            raise Failure(key + " must be an integer")
    if not (config["quota_gib"] > 0 and 0 < config["low_pct"] < config["high_pct"] <= 100
            and config["batch_hours"] > 0 and 0 < config["max_pass_pct"] < 100):
        raise Failure("invalid quota, water marks or pass bounds")
    for key in ("bucket", "alias"):
        if not re.fullmatch(r"[a-zA-Z0-9][a-zA-Z0-9._-]*", config[key]):
            raise Failure("invalid " + key)
    if not re.fullmatch(r"[a-zA-Z0-9._-]+(?:/[a-zA-Z0-9._-]+)*", config["trail_prefix"]):
        raise Failure("invalid trail prefix")
    if any(part in (".", "..") for part in config["trail_prefix"].split("/")):
        raise Failure("unsafe trail prefix")


def oldest_groups(rows, trail_prefix):
    groups = {}
    seen = set()
    for row in rows:
        key, size, version = row.get("key"), row.get("size"), row.get("versionId")
        if not isinstance(key, str):
            raise Failure("listing has no object key")
        if key == trail_prefix or key.startswith(trail_prefix + "/"):
            continue
        match = re.fullmatch(r"([a-zA-Z0-9._-]+)/(\d{4})/(\d{2})/(\d{2})/(\d{2})/(.+)", key)
        if not match or any(part in (".", "..") for part in key.split("/")):
            raise Failure("unexpected audit object shape; refusing to guess deletion scope")
        try:
            stamp = dt.datetime(*map(int, match.groups()[1:5]))
        except ValueError:
            raise Failure("invalid date in audit object key") from None
        if type(size) is not int or size < 0 or not isinstance(version, str) or not version:
            raise Failure("version listing lacks a valid size or version ID")
        if (key, version) in seen:
            raise Failure("duplicate object version in listing")
        seen.add((key, version))
        prefix = "/".join(key.split("/")[:5])
        group = groups.setdefault(prefix, {"prefix": prefix, "stamp": stamp,
                                          "size": 0, "versions": []})
        group["size"] += size
        group["versions"].append({"key": key, "version": version, "size": size})
    return sorted(groups.values(), key=lambda group: (group["stamp"], group["prefix"]))


def metrics(config, used, prefixes, freed, result, total, over_high=0):
    directory = Path(config["textfile_dir"])
    if not directory.is_dir():
        raise Failure("node exporter textfile directory is missing")
    values = {
        "bucket_used_bytes": ("gauge", used), "quota_bytes": ("gauge", config["quota_gib"] * 2**30),
        "over_high_water": ("gauge", over_high),
        "objects_total": ("gauge", prefixes), "bytes_total": ("gauge", freed),
        "hour_prefixes_total": ("counter", total),
        "last_run_timestamp_seconds": ("gauge", int(time.time())),
        "last_result": ("gauge", result),
    }
    lines = []
    for suffix, (kind, value) in values.items():
        name = "goat_audit_evict_" + suffix
        lines += ["# HELP " + name + " audit middle-tier eviction measurement",
                  "# TYPE " + name + " " + kind,
                  name + '{bucket="' + config["bucket"] + '"} ' + str(value)]
    fd, name = tempfile.mkstemp(prefix=".audit-fifo-", dir=directory)
    # node_exporter reads the textfile directory as an unprivileged user, so the
    # (non-secret) metrics file is world-readable; mkstemp creates it 0600.
    os.fchmod(fd, 0o644)
    with os.fdopen(fd, "w") as stream:
        stream.write("\n".join(lines) + "\n")
    os.replace(name, directory / "audit_fifo_evict.prom")


def run(config, backend, evict=False):
    validate(config)
    directory = Path(config["state_dir"])
    state_path, pending = directory / "state.json", directory / "pending-pass.json"
    state = json.loads(state_path.read_text()) if state_path.exists() else {
        "bucket": config["bucket"], "prefixes_total": 0}
    if state.get("bucket") != config["bucket"]:
        raise Failure("persisted eviction count belongs to a different bucket")
    total = state["prefixes_total"]
    if type(total) is not int or total < 0:
        raise Failure("invalid persisted eviction count")
    target = config["alias"] + "/" + config["bucket"]
    quota = config["quota_gib"] * 2**30
    used, prefixes, freed, result = 0, 0, 0, 1
    journal = None
    over = lambda: 1 if used >= quota * config["high_pct"] // 100 else 0
    try:
        used = backend.used(target)
        result = 3
        if not evict:
            # The finally block rewrites the textfile with `result`; left at 3
            # here, every measure-only pass would publish last_result 3 and
            # trip AuditEvictorOperationFailed. It must carry the outcome.
            result = 4 if over() else 0
            if result == 4:
                # Observation of a breached threshold is not a success: the
                # bucket is at or past high-water, eviction is disabled, and
                # uploads will be refused at the quota. Exit non-zero so the
                # timer unit fails loudly instead of printing result=0.
                print("audit-fifo-evict: WARNING used is at or over the "
                      "high-water mark and eviction is DISABLED (measure-only)")
                return 4
            return 0
        # A previous pass must be auditable before a new destructive pass.
        # A journal surviving a crash records both confirmed and pending actions.
        if pending.exists():
            previous = json.loads(pending.read_text())
            if not previous["trail_key"].startswith(config["trail_prefix"] + "/"):
                raise Failure("pending trail target is outside the configured trail")
            backend.trail(pending, target + "/" + previous["trail_key"])
            pending.unlink()
        if used < quota * config["high_pct"] // 100:
            result = 0
            return 0
        budget = used * config["max_pass_pct"] // 100
        limit = max(quota * config["low_pct"] // 100, used - budget)
        groups = oldest_groups(backend.versions(target), config["trail_prefix"])
        stamp = dt.datetime.now(dt.timezone.utc).strftime("%Y/%m/%d")
        journal = {"event": "AuditMiddleTierEviction", "bucket": config["bucket"],
                   "trail_key": config["trail_prefix"] + "/" + stamp + "/" + str(uuid.uuid4()) + ".json",
                   "used_before_bytes": used, "byte_budget": budget, "attempts": []}
        charged = 0
        for group in groups:
            if used <= limit or prefixes >= config["batch_hours"]:
                break
            if group["size"] > budget - charged:
                # Never leap over an oversized oldest hour to delete newer data.
                break
            before = used
            for version in group["versions"]:
                attempt = {**version, "confirmed": False}
                journal["attempts"].append(attempt)
                atomic_json(pending, journal)
                backend.remove(target + "/" + version["key"], version["version"])
                charged += version["size"]
                attempt["confirmed"] = True
                atomic_json(pending, journal)
            prefixes += 1
            total += 1
            atomic_json(state_path, {"bucket": config["bucket"], "prefixes_total": total})
            used = backend.used(target)
            freed += max(0, before - used)
            if group["size"] and used >= before:
                raise Failure("version removals produced no measured quota relief")
        result = 0 if used <= quota * config["low_pct"] // 100 else 2
    except Failure as error:
        if result == 1:
            error.code = 1
        raise
    finally:
        # Persist failures too. A failed trail remains on disk and must upload
        # before another destructive pass; it never becomes an accepted rc=2.
        try:
            if journal is not None and journal["attempts"]:
                journal.update(data_used_after_bytes=used, result_before_trail=result)
                atomic_json(pending, journal)
                backend.trail(pending, target + "/" + journal["trail_key"])
                pending.unlink()
                used = backend.used(target)
                freed = max(0, journal["used_before_bytes"] - used)
                if result in (0, 2):
                    result = 0 if used <= config["quota_gib"] * 2**30 * config["low_pct"] // 100 else 2
        except Failure:
            result = 3
            metrics(config, used, prefixes, freed, result, total, over_high=over())
            raise
        try:
            metrics(config, used, prefixes, freed, result, total, over_high=over())
        except (OSError, Failure):
            # If publication recovers, expose the failed operation rather than
            # the measurement outcome. A persistent failure leaves the prior
            # timestamp intact so the stale-metrics alert can detect it.
            metrics(config, used, prefixes, freed, 3, total, over_high=over())
            raise
    return result


def main():
    parser = Parser(description=__doc__)
    parser.add_argument("--config", required=True, type=Path)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--measure-only", action="store_true")
    mode.add_argument("--evict", action="store_true")
    try:
        args = parser.parse_args()
        config = json.loads(args.config.read_text())
        with (Path(config["state_dir"]) / ".evict.lock").open("w") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            result = run(config, MC(config), evict=args.evict)
        print("audit-fifo-evict: result=" + str(result))
        return result
    except Failure as error:
        print("audit-fifo-evict: FAILED " + str(error))
        return error.code
    except (OSError, ValueError, KeyError, TypeError) as error:
        print("audit-fifo-evict: FAILED " + str(error))
        return 3


if __name__ == "__main__":
    raise SystemExit(main())
