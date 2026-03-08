# Test Scripts

This directory contains integration test scripts for NuAnalytics.

## MCP Server Test

`test_mcp_server.py` - Tests the MCP server implementation by simulating a full client session.

### Prerequisites

- Python 3.7+
- NuAnalytics built with MCP feature:
  ```bash
  cargo build --features mcp
  ```

### Usage

```bash
# Run with built-in test YAML
python3 tests/scripts/test_mcp_server.py

# Run with a custom YAML file
python3 tests/scripts/test_mcp_server.py --yaml-file samples/degrees/my-degree.yaml

# Verbose output (show full responses)
python3 tests/scripts/test_mcp_server.py -v
```

### What It Tests

1. **Initialize Handshake** - Verifies MCP protocol initialization
2. **List Tools** - Confirms `get_degree_schema` and `validate_degree` are registered
3. **Get Schema** - Calls `get_degree_schema` and verifies response
4. **Validate Degree** - Calls `validate_degree` with test YAML and shows results

### Example Output

```
============================================================
  Test 4: Call validate_degree
============================================================

✓ PASS: validate_degree
       Valid: True, Errors: 0, Warnings: 0
       Degree: Bachelor of Science in Computer Science (Boston)
       Courses: 25
       Requirements: 7

============================================================
  Summary
============================================================

✓ All tests passed!
```

### Exit Codes

- `0` - All tests passed
- `1` - One or more tests failed

### Troubleshooting

**Server won't start:**
- Ensure you've built with MCP feature: `cargo build --features mcp`
- Check that no other process is using stdin/stdout

**Tests fail with timeout:**
- The server may be slow to start on first run (compilation)
- Try running `cargo build --features mcp` first

**YAML validation errors:**
- Check that your YAML follows the schema (run `get_degree_schema` for docs)
- Ensure courses have required fields: `title`, `prefix`, `number`, `credits`
