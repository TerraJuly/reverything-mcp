"""Test real MCP stdio using exclusively generated synthetic fixtures.

No project source or personal files are searched. Query results and actual paths
are never printed. Temporary fixtures are removed after the server exits.

Usage: python scripts/smoke_test.py [--require-everything | --expect-unavailable] [server.exe]
Requires Python 3.10+, with no third-party Python packages.
"""

import argparse
import json
import pathlib
import queue
import subprocess
import tempfile
import threading
import time

ROOT = pathlib.Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("executable", nargs="?", default=str(ROOT / "target/debug/reverything-mcp.exe"))
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--require-everything", action="store_true")
    mode.add_argument("--expect-unavailable", action="store_true")
    options = parser.parse_args()
    # Only our generated directory is in scope for live search/info requests.
    # System TEMP can be excluded from Everything's index. Use an isolated,
    # git-ignored fixture directory, never the project's source directories.
    fixture_parent = ROOT / ".test-fixtures"
    fixture_parent.mkdir(exist_ok=True)
    fixtures = tempfile.TemporaryDirectory(prefix="synthetic-", dir=fixture_parent)
    fixture_root = pathlib.Path(fixtures.name)
    for name in [
        "main.rs", "test_alpha.rs", "test_beta.rs", "config.toml", "example.png", "example [1].rs",
        "note.txt", "sample.mp3", "sample.mp4", "sample.zip", "sample.msi",
    ]:
        (fixture_root / name).write_text("synthetic MCP test fixture\n", encoding="utf-8")
    (fixture_root / "nested").mkdir()
    (fixture_root / "nested/nested.rs").write_text("synthetic nested fixture\n", encoding="utf-8")
    process = None
    try:
        process = subprocess.Popen(
            [str(pathlib.Path(options.executable).resolve())], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
            stderr=subprocess.PIPE, text=True, encoding="utf-8", bufsize=1,
        )
        responses = queue.Queue()

        def read_stdout():
            try:
                for line in process.stdout:
                    responses.put(json.loads(line))
            except Exception:
                responses.put(ValueError("Invalid MCP JSON on stdout; response omitted for privacy"))
            finally:
                responses.put(EOFError("MCP stdout closed"))

        def read_stderr():
            # Drain diagnostics to prevent pipe deadlock, but never print paths.
            for _ in process.stderr:
                pass

        threading.Thread(target=read_stdout, daemon=True).start()
        threading.Thread(target=read_stderr, daemon=True).start()
        request_id = 0

        def send(method, params=None, notification=False):
            nonlocal request_id
            message = {"jsonrpc": "2.0", "method": method}
            if params is not None:
                message["params"] = params
            if not notification:
                request_id += 1
                message["id"] = request_id
            process.stdin.write(json.dumps(message) + "\n")
            process.stdin.flush()
            if notification:
                return None
            while True:
                try:
                    response = responses.get(timeout=20)
                except queue.Empty as error:
                    raise TimeoutError("MCP response timed out") from error
                if isinstance(response, Exception):
                    raise response
                if response.get("id") == request_id:
                    return response

        def call(name, arguments):
            response = send("tools/call", {"name": name, "arguments": arguments})
            assert "error" not in response, "Unexpected MCP protocol error; details omitted"
            return response["result"]

        def data_of(result):
            assert not result.get("isError"), "Unexpected tool error; details omitted"
            return result.get("structuredContent") or json.loads(result["content"][0]["text"])

        def scoped_files(**filters):
            # Caller cannot override path, so every actual search stays isolated.
            assert "path" not in filters
            return data_of(call("search_files", {**filters, "path": str(fixture_root)}))

        initialization = send("initialize", {
            "protocolVersion": "2025-06-18", "capabilities": {},
            "clientInfo": {"name": "reverything-smoke-test", "version": "1.0"},
        })
        assert initialization["result"]["serverInfo"]["name"] == "reverything-mcp"
        send("notifications/initialized", notification=True)
        tools = send("tools/list")["result"]["tools"]
        assert {tool["name"] for tool in tools} == {
            "search_files", "search_folders", "get_file_info", "get_search_syntax", "get_status"
        }
        for tool in tools:
            assert tool["annotations"]["readOnlyHint"] is True
            assert tool["annotations"]["destructiveHint"] is False
        print("PASS: initialize, five tools, and read-only annotations")
        syntax = data_of(call("get_search_syntax", {}))
        assert "name_regex" in syntax["guide"]
        print("PASS: offline search syntax tool")

        invalid = call("search_files", {"query": " ", "limit": 20})
        assert invalid.get("isError") is True
        print("PASS: validation errors are visible MCP tool errors")
        status = call("get_status", {})
        if status.get("isError"):
            assert not options.require_everything, "Everything unavailable; diagnostic text omitted"
            if options.expect_unavailable:
                error = status["content"][0]["text"]
                assert "https://www.voidtools.com/downloads/" in error
                assert "安装" in error and "启动" in error
                failed = call("search_files", {"path": str(fixture_root), "extensions": ["rs"], "limit": 1})
                assert failed.get("isError") is True
                assert "https://www.voidtools.com/downloads/" in failed["content"][0]["text"]
                print("PASS: missing client gives actionable installation hints without breaking MCP")
            else:
                print("SKIP: live synthetic-fixture tests; Everything unavailable")
        else:
            assert not options.expect_unavailable, "Expected unavailable Everything instance"
            status_data = data_of(status)
            assert status_data["es_version"] == "1.1.1.40" and status_data["index_ready"] is True
            print("PASS: local connection and index readiness")
            # Allow Everything's filesystem change monitor to index our fixtures.
            deadline = time.monotonic() + 10
            while True:
                filtered = scoped_files(extensions=["rs"], name_regex=r"^main\.rs$", recursive=False)
                if filtered["returned"] == 1:
                    break
                if time.monotonic() >= deadline:
                    raise AssertionError("Synthetic fixtures were not indexed within the test deadline")
                time.sleep(0.1)
            assert filtered["results"][0]["is_directory"] is False
            print("PASS: synthetic path, extension, filename regex, and direct-child filters")

            empty = scoped_files(query="__synthetic_nonexistent_name__")
            assert empty["results"] == [] and empty["next_offset"] is None and not empty["has_more"]
            print("PASS: empty result within synthetic directory")
            page = scoped_files(extensions=["rs"], limit=1)
            assert page["returned"] == 1 and page["has_more"] and page["next_offset"] == 1
            second = scoped_files(extensions=["rs"], limit=1, offset=1)
            assert second["returned"] == 1
            assert second["results"][0]["full_path"] != page["results"][0]["full_path"]
            all_files = scoped_files(extensions=["rs"], limit=200)
            assert all_files["returned"] == 5
            assert not all_files["has_more"]
            assert all(entry["full_path"].casefold().startswith(str(fixture_root).casefold() + "\\") for entry in all_files["results"])
            print("PASS: synthetic JSON metadata and multi-page results")

            folders = data_of(call("search_folders", {"path": str(fixture_root), "recursive": False}))
            assert folders["returned"] == 1 and folders["results"][0]["is_directory"]
            print("PASS: synthetic directory-only search")
            for filename, directory in [("main.rs", False), ("example [1].rs", False), ("nested", True)]:
                info = data_of(call("get_file_info", {"path": str(fixture_root / filename)}))
                assert info["found"] and info["entry"]["is_directory"] is directory
            absent = data_of(call("get_file_info", {"path": str(fixture_root / "__synthetic_missing_file__")}))
            assert not absent["found"] and absent["entry"] is None
            print("PASS: exact synthetic file/directory metadata and missing fixture")

            for category, filename in {
                "image": "example.png", "document": "note.txt", "audio": "sample.mp3",
                "video": "sample.mp4", "archive": "sample.zip", "executable": "sample.msi",
            }.items():
                matches = scoped_files(category=category)
                assert matches["returned"] == 1, "Synthetic category count mismatch"
                assert matches["results"][0]["full_path"].endswith(filename)
            for malformed in [
                {"query": "foo> | folder:"}, {"extensions": ["rs;folder:"]},
                {"recursive": False, "query": "main"},
            ]:
                # All are rejected before an ES query can run.
                assert call("search_files", malformed).get("isError") is True
            print("PASS: synthetic category filter and query validation")
    finally:
        if process is not None:
            if process.stdin is not None:
                process.stdin.close()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)
            code = process.returncode
        else:
            code = None
        fixtures.cleanup()
        if code not in (None, 0):
            raise RuntimeError("Server exited unsuccessfully; diagnostic paths are omitted")


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        # Tracebacks and OS error strings can contain account/directory names.
        # The preceding PASS lines identify the stage without leaking paths.
        print(f"FAIL: {type(error).__name__}; diagnostic paths and metadata omitted for privacy")
        raise SystemExit(1) from None
