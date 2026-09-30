#!/usr/bin/env python3
"""
MCP Server Integration Test Script

Tests the NuAnalytics MCP server by simulating a full client session:
1. Initialize handshake
2. The tools served, against the expected set (and import_degree only with --allow-writes)
3. get_reference, validate_degree and audit_degree, inline and by sample reference
4. Failures: an unknown source and an unknown argument come back as isError envelopes
5. With --with-db: the database tools against the configured backend

Usage:
    python3 tests/scripts/test_mcp_server.py [--yaml-file PATH] [--binary PATH] [--with-db]

Requirements:
    - Python 3.7+
    - NuAnalytics built with the mcp feature (the default)
"""

import argparse
import json
import subprocess
import sys
import os
from pathlib import Path


# Default test YAML for validation
DEFAULT_TEST_YAML = """degree:
  id: test-degree
  institution: Test University
  program: Test Program
  total_credits: 120
  gpa_minimum: 2.0

requirements:
  intro:
    name: Introduction
    type: all
    category: major
    courses:
      - CS101
      - CS102

courses:
  CS101:
    title: Intro to CS
    prefix: CS
    number: "101"
    credits: 4

  CS102:
    title: Data Structures
    prefix: CS
    number: "102"
    credits: 4
    prerequisites_raw: "CS101"
"""


class McpTestClient:
    """Simple MCP client for testing."""

    def __init__(self, server_command: list[str], cwd: str = None):
        self.proc = subprocess.Popen(
            server_command,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            cwd=cwd,
        )
        self.request_id = 0

    def send_request(self, method: str, params: dict = None) -> dict:
        """Send a JSON-RPC request and return the response."""
        self.request_id += 1
        request = {
            "jsonrpc": "2.0",
            "id": self.request_id,
            "method": method,
        }
        if params is not None:
            request["params"] = params

        self.proc.stdin.write(json.dumps(request) + "\n")
        self.proc.stdin.flush()

        response_line = self.proc.stdout.readline()
        if not response_line:
            raise RuntimeError("No response from server")

        return json.loads(response_line)

    def send_notification(self, method: str, params: dict = None):
        """Send a JSON-RPC notification (no response expected)."""
        notification = {"jsonrpc": "2.0", "method": method}
        if params is not None:
            notification["params"] = params

        self.proc.stdin.write(json.dumps(notification) + "\n")
        self.proc.stdin.flush()

    def initialize(self) -> dict:
        """Perform MCP initialization handshake."""
        response = self.send_request(
            "initialize",
            {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "test-client", "version": "1.0.0"},
            },
        )
        self.send_notification("notifications/initialized")
        return response

    def list_tools(self) -> dict:
        """List available tools."""
        return self.send_request("tools/list", {})

    def call_tool(self, name: str, arguments: dict) -> dict:
        """Call a tool with arguments."""
        return self.send_request(
            "tools/call", {"name": name, "arguments": arguments}
        )

    def close(self):
        """Terminate the server process."""
        self.proc.terminate()
        try:
            self.proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.proc.kill()


def find_project_root() -> Path:
    """Find the NuAnalytics project root directory."""
    # Start from script location and walk up
    current = Path(__file__).resolve().parent
    while current != current.parent:
        if (current / "Cargo.toml").exists():
            return current
        current = current.parent
    raise RuntimeError("Could not find project root (no Cargo.toml found)")


def print_section(title: str):
    """Print a section header."""
    print(f"\n{'=' * 60}")
    print(f"  {title}")
    print(f"{'=' * 60}\n")


def print_result(label: str, success: bool, details: str = ""):
    """Print a test result."""
    status = "✓ PASS" if success else "✗ FAIL"
    print(f"{status}: {label}")
    if details:
        print(f"       {details}")


# Every tool a read-only server serves; `import_degree` is added by --allow-writes.
EXPECTED_TOOLS = {
    "get_reference", "validate_degree", "audit_degree", "find_courses_matching",
    "get_course_detail", "convert_degree", "trim_degree", "analyze_degree",
    "render_degree_report", "render_plan_graph", "list_sample_degrees",
    "search_institutions", "search_cip_codes", "get_lookup_codes",
    "get_completion_demographics", "search_degrees", "get_degree", "get_stored_analysis",
    "render_stored_report", "compare_degrees", "query_sql",
}


def tool_text(response: dict) -> tuple:
    """The (is_error, parsed JSON or raw text) of a tools/call response."""
    result = response.get("result", {})
    text = "".join(c.get("text", "") for c in result.get("content", []))
    try:
        body = json.loads(text)
    except json.JSONDecodeError:
        body = text
    return result.get("isError", False), body


class Checks:
    """Runs named checks and remembers whether all passed."""

    def __init__(self, verbose: bool):
        self.verbose = verbose
        self.all_passed = True

    def check(self, label: str, fn):
        try:
            ok, details = fn()
        except Exception as e:  # a crashed check is a failed one
            ok, details = False, f"{type(e).__name__}: {e}"
        print_result(label, ok, details)
        self.all_passed = self.all_passed and ok


def server_command(project_root: Path, binary: str, allow_writes: bool) -> list:
    cmd = [binary, "mcp"] if binary else ["cargo", "run", "--quiet", "--", "mcp"]
    return cmd + (["--allow-writes"] if allow_writes else [])


def run_session(project_root: Path, args, test_yaml: str, checks: Checks):
    client = McpTestClient(server_command(project_root, args.binary, False), cwd=str(project_root))
    try:
        print_section("Handshake and tool list")

        def initialize():
            result = client.initialize().get("result", {})
            ok = "tools" in result.get("capabilities", {}) and "instructions" in result
            return ok, f"protocol {result.get('protocolVersion', 'unknown')}"
        checks.check("initialize", initialize)

        def tools_served():
            names = {t["name"] for t in client.list_tools().get("result", {}).get("tools", [])}
            missing, extra = EXPECTED_TOOLS - names, names - EXPECTED_TOOLS
            return not missing and not extra, f"{len(names)} tools; missing {sorted(missing)}, unexpected {sorted(extra)}"
        checks.check("read-only server serves exactly the expected tools", tools_served)

        print_section("Degree tools")

        def reference():
            err, body = tool_text(client.call_tool("get_reference", {"topic": "degree-yaml", "section": "degree"}))
            text = json.dumps(body)
            return not err and "degree" in text.lower(), f"{len(text)} chars"
        checks.check("get_reference(topic=degree-yaml)", reference)

        def validate_inline():
            err, body = tool_text(client.call_tool("validate_degree", {"content": test_yaml}))
            ok = not err and isinstance(body, dict) and "is_valid" in body
            handle = body.get("source", {}).get("handle") if isinstance(body, dict) else None
            return ok and bool(handle), f"valid={body.get('is_valid')}, errors={len(body.get('errors', []))}, handle={handle}"
        checks.check("validate_degree(content=…) caches and returns a handle", validate_inline)

        def validate_sample():
            err, body = tool_text(client.call_tool("validate_degree", {"degree": "sample:csu"}))
            return not err and body.get("source", {}).get("kind") == "sample", f"valid={body.get('is_valid')}"
        checks.check("validate_degree(degree=sample:csu)", validate_sample)

        def audit():
            err, body = tool_text(client.call_tool("audit_degree", {"content": test_yaml}))
            return not err and isinstance(body, dict), f"deep chains: {len(body.get('deep_chains', []))}"
        checks.check("audit_degree(content=…)", audit)

        print_section("Failures")

        def unknown_source():
            err, body = tool_text(client.call_tool("validate_degree", {"degree": "sample:nope"}))
            code = body.get("error", {}).get("code") if isinstance(body, dict) else None
            return err and code == "source_not_found", f"isError={err}, code={code}"
        checks.check("an unknown sample is an isError envelope", unknown_source)

        def unknown_argument():
            err, body = tool_text(client.call_tool("validate_degree", {"yaml_content": test_yaml}))
            code = body.get("error", {}).get("code") if isinstance(body, dict) else None
            return err and code == "bad_arguments", f"isError={err}, code={code}"
        checks.check("an argument the tool does not take is refused", unknown_argument)

        if args.with_db:
            print_section("Database tools")
            db_checks(client, checks)
    finally:
        client.close()

    writes = McpTestClient(server_command(project_root, args.binary, True), cwd=str(project_root))
    try:
        writes.initialize()

        def served_with_writes():
            names = {t["name"] for t in writes.list_tools().get("result", {}).get("tools", [])}
            return names == EXPECTED_TOOLS | {"import_degree"}, f"{len(names)} tools"
        checks.check("--allow-writes adds exactly import_degree", served_with_writes)
    finally:
        writes.close()


def db_checks(client: McpTestClient, checks: Checks):
    """The database tools, against whatever backend the config names."""
    found = {}

    def search():
        err, body = tool_text(client.call_tool("search_degrees", {"limit": 1}))
        programs = body.get("programs", []) if isinstance(body, dict) else []
        if programs:
            found["key"] = programs[0]["program_key"]
        return not err and bool(programs), f"first: {found.get('key')}"
    checks.check("search_degrees", search)

    def get_degree():
        err, body = tool_text(client.call_tool("get_degree", {"program_key": found["key"]}))
        return not err and "stored_runs" in body, f"{len(body.get('stored_runs', []))} stored runs"
    checks.check("get_degree(program_key=…)", get_degree)

    def stored_analysis():
        err, body = tool_text(client.call_tool("get_stored_analysis", {"degree": found["key"]}))
        return not err and body.get("count", 0) >= 1, f"{body.get('count')} runs"
    checks.check("get_stored_analysis(degree=…)", stored_analysis)

    def demographics():
        err, body = tool_text(client.call_tool(
            "get_completion_demographics",
            {"group_by": "school", "cip_prefix": "11.", "carnegie_class": 15, "limit": 3}))
        return not err and body.get("group_by") == "school", f"year {body.get('filters', {}).get('year')}"
    checks.check("get_completion_demographics(group_by=school)", demographics)

    def sql():
        err, body = tool_text(client.call_tool(
            "query_sql",
            {"sql": "SELECT count(*) AS n FROM institutions WHERE state = $1->>'state'",
             "params": {"state": "MA"}}))
        return not err and body.get("count") == 1, f"rows: {body.get('rows')}"
    checks.check("query_sql with params", sql)

    def sql_write_refused():
        err, body = tool_text(client.call_tool("query_sql", {"sql": "DELETE FROM programs"}))
        code = body.get("error", {}).get("code") if isinstance(body, dict) else None
        return err and code == "sql_rejected", f"code={code}"
    checks.check("query_sql refuses a write", sql_write_refused)


def main():
    parser = argparse.ArgumentParser(description="Test the NuAnalytics MCP server")
    parser.add_argument("--yaml-file", type=str,
                        help="A YAML file to validate (the built-in test YAML otherwise)")
    parser.add_argument("--binary", type=str,
                        help="A built nuanalytics binary (cargo run otherwise)")
    parser.add_argument("--with-db", action="store_true",
                        help="Also call the database tools against the configured backend")
    parser.add_argument("--verbose", "-v", action="store_true", help="Show full responses")
    args = parser.parse_args()

    project_root = find_project_root()
    print(f"Project root: {project_root}")
    if args.yaml_file:
        test_yaml = Path(args.yaml_file).read_text()
        print(f"Using YAML file: {args.yaml_file}")
    else:
        test_yaml = DEFAULT_TEST_YAML

    checks = Checks(args.verbose)
    run_session(project_root, args, test_yaml, checks)

    print_section("Summary")
    if checks.all_passed:
        print("✓ All tests passed!")
        sys.exit(0)
    print("✗ Some tests failed")
    sys.exit(1)


if __name__ == "__main__":
    main()
