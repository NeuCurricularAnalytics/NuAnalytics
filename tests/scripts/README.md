# Test Scripts

## MCP server test

`test_mcp_server.py` drives the MCP server over stdio as a client would, and checks:

1. the initialize handshake;
2. the tools served, against the expected set — and `import_degree` only under
   `--allow-writes`;
3. `get_reference`, `validate_degree` and `audit_degree`, with the degree inline and as a
   `sample:` reference;
4. failures: an unknown source and an unknown argument come back as `isError` envelopes;
5. with `--with-db`, the database tools against the configured backend — including that a
   stored program is read from its stored run, that fresh-run settings on one are refused
   without `fresh=true`, and that `query_sql` refuses a write.

It needs Python 3.7+ and a build with the `mcp` feature (the default).

```bash
python3 tests/scripts/test_mcp_server.py                          # cargo run, built-in degree
python3 tests/scripts/test_mcp_server.py --binary target/release/nuanalytics
python3 tests/scripts/test_mcp_server.py --yaml-file samples/degrees/neu-khoury-bscs-boston.yaml
python3 tests/scripts/test_mcp_server.py --with-db                # needs `db login` first
python3 tests/scripts/test_mcp_server.py -v                       # print full responses
```

It exits 0 when every check passes and 1 otherwise.

**If the server does not start,** build first (`cargo build`) or pass `--binary`; the
first `cargo run` compiles and can time out. **If validation fails on your own file,**
check it against the degree format (`get_reference(topic="degree-yaml")`, or
[docs/degree.md](../../docs/degree.md#the-degree-format)): courses need `title`, `prefix`
(or `subject`), `number` and `credits`.
