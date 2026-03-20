#!/usr/bin/env python3
"""
MCP Server Integration Test Script

Tests the NuAnalytics MCP server by simulating a full client session:
1. Initialize handshake
2. List available tools
3. Call get_degree_schema
4. Call validate_degree with test YAML

Usage:
    python3 tests/scripts/test_mcp_server.py [--yaml-file PATH]

Requirements:
    - Python 3.7+
    - NuAnalytics built with MCP feature: cargo build --features mcp
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


def main():
    parser = argparse.ArgumentParser(
        description="Test the NuAnalytics MCP server"
    )
    parser.add_argument(
        "--yaml-file",
        type=str,
        help="Path to a YAML file to validate (uses built-in test YAML if not provided)",
    )
    parser.add_argument(
        "--verbose", "-v", action="store_true", help="Show full responses"
    )
    args = parser.parse_args()

    # Find project root
    try:
        project_root = find_project_root()
    except RuntimeError as e:
        print(f"Error: {e}", file=sys.stderr)
        sys.exit(1)

    print(f"Project root: {project_root}")

    # Load test YAML
    if args.yaml_file:
        yaml_path = Path(args.yaml_file)
        if not yaml_path.exists():
            print(f"Error: YAML file not found: {yaml_path}", file=sys.stderr)
            sys.exit(1)
        test_yaml = yaml_path.read_text()
        print(f"Using YAML file: {yaml_path}")
    else:
        test_yaml = DEFAULT_TEST_YAML
        print("Using built-in test YAML")

    # Start MCP server
    print_section("Starting MCP Server")
    server_cmd = ["cargo", "run", "--features", "mcp", "--", "mcp"]
    print(f"Command: {' '.join(server_cmd)}")

    try:
        client = McpTestClient(server_cmd, cwd=str(project_root))
    except Exception as e:
        print(f"Error starting server: {e}", file=sys.stderr)
        sys.exit(1)

    all_passed = True

    try:
        # Test 1: Initialize
        print_section("Test 1: Initialize Handshake")
        try:
            response = client.initialize()
            has_tools = "tools" in response.get("result", {}).get("capabilities", {})
            has_instructions = "instructions" in response.get("result", {})
            success = has_tools and has_instructions
            print_result(
                "Initialize handshake",
                success,
                f"Protocol: {response.get('result', {}).get('protocolVersion', 'unknown')}",
            )
            if args.verbose:
                print(json.dumps(response, indent=2))
            all_passed = all_passed and success
        except Exception as e:
            print_result("Initialize handshake", False, str(e))
            all_passed = False

        # Test 2: List Tools
        print_section("Test 2: List Tools")
        try:
            response = client.list_tools()
            tools = response.get("result", {}).get("tools", [])
            tool_names = [t["name"] for t in tools]
            has_schema_tool = "get_degree_schema" in tool_names
            has_validate_tool = "validate_degree" in tool_names
            has_audit_tool = "audit_degree" in tool_names
            success = has_schema_tool and has_validate_tool and has_audit_tool
            print_result(
                "List tools",
                success,
                f"Found tools: {', '.join(tool_names)}",
            )
            if args.verbose:
                print(json.dumps(response, indent=2))
            all_passed = all_passed and success
        except Exception as e:
            print_result("List tools", False, str(e))
            all_passed = False

        # Test 3: Get Schema
        print_section("Test 3: Call get_degree_schema")
        try:
            response = client.call_tool("get_degree_schema", {"section": "degree"})
            content = response.get("result", {}).get("content", [])
            has_content = len(content) > 0 and content[0].get("type") == "text"
            text = content[0].get("text", "") if has_content else ""
            has_degree_info = "Degree Metadata" in text or "degree" in text.lower()
            success = has_content and has_degree_info
            print_result(
                "get_degree_schema(section='degree')",
                success,
                f"Response length: {len(text)} chars",
            )
            if args.verbose:
                print(text[:500] + "..." if len(text) > 500 else text)
            all_passed = all_passed and success
        except Exception as e:
            print_result("get_degree_schema", False, str(e))
            all_passed = False

        # Test 4: Validate Degree
        print_section("Test 4: Call validate_degree")
        try:
            response = client.call_tool(
                "validate_degree", {"yaml_content": test_yaml}
            )
            content = response.get("result", {}).get("content", [])
            has_content = len(content) > 0 and content[0].get("type") == "text"

            if has_content:
                validation_result = json.loads(content[0]["text"])
                is_valid = validation_result.get("is_valid", False)
                errors = validation_result.get("errors", [])
                warnings = validation_result.get("warnings", [])
                context = validation_result.get("context", {})

                print_result(
                    "validate_degree",
                    True,
                    f"Valid: {is_valid}, Errors: {len(errors)}, Warnings: {len(warnings)}",
                )
                print(f"       Degree: {context.get('degree_name', 'unknown')}")
                print(f"       Courses: {context.get('total_courses', 0)}")
                print(f"       Requirements: {context.get('total_requirements', 0)}")

                if errors:
                    print("\n       Errors:")
                    for err in errors[:3]:  # Show first 3
                        print(f"         - {err.get('error_type')}: {err.get('message')[:60]}...")

                if warnings:
                    print("\n       Warnings:")
                    for warn in warnings[:3]:  # Show first 3
                        print(f"         - {warn.get('warning_type')}: {warn.get('message')[:60]}...")

                if args.verbose:
                    print("\n       Full response:")
                    print(json.dumps(validation_result, indent=2))
            else:
                print_result("validate_degree", False, "No content in response")
                all_passed = False

        except json.JSONDecodeError as e:
            print_result("validate_degree", False, f"Invalid JSON response: {e}")
            all_passed = False
        except Exception as e:
            print_result("validate_degree", False, str(e))
            all_passed = False

        # Test 5: Audit Degree
        print_section("Test 5: Call audit_degree")
        try:
            response = client.call_tool(
                "audit_degree", {"yaml_content": test_yaml}
            )
            content = response.get("result", {}).get("content", [])
            has_content = len(content) > 0 and content[0].get("type") == "text"

            if has_content:
                audit_result = json.loads(content[0]["text"])
                passed = audit_result.get("passed", False)
                val_errors = audit_result.get("validation_errors", 0)
                missing = audit_result.get("missing_prerequisites", [])
                chains = audit_result.get("deep_chains", [])

                print_result(
                    "audit_degree",
                    True,
                    f"Passed: {passed}, Errors: {val_errors}, Missing prereqs: {len(missing)}, Deep chains: {len(chains)}",
                )
                print(f"       Degree: {audit_result.get('degree_name', 'unknown')}")
                print(f"       Courses: {audit_result.get('total_courses', 0)}")

                if args.verbose:
                    print("\n       Full response:")
                    print(json.dumps(audit_result, indent=2))
            else:
                print_result("audit_degree", False, "No content in response")
                all_passed = False

        except json.JSONDecodeError as e:
            print_result("audit_degree", False, f"Invalid JSON response: {e}")
            all_passed = False
        except Exception as e:
            print_result("audit_degree", False, str(e))
            all_passed = False

    finally:
        client.close()

    # Summary
    print_section("Summary")
    if all_passed:
        print("✓ All tests passed!")
        sys.exit(0)
    else:
        print("✗ Some tests failed")
        sys.exit(1)


if __name__ == "__main__":
    main()
