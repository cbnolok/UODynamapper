#!/usr/bin/env python3
"""Save read-only evidence from an already analyzed ghidra-mcp program."""

import argparse
import json
import os
from pathlib import Path
from urllib.parse import urlencode
from urllib.request import urlopen


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--url", default=os.environ.get("GHIDRA_MCP_URL"),
                        help="ghidra-mcp HTTP URL, or set GHIDRA_MCP_URL")
    parser.add_argument("--program", required=True, help="explicit open program name")
    parser.add_argument("--output", required=True, type=Path, help="new JSON evidence file")
    subparsers = parser.add_subparsers(dest="command", required=True)
    functions = subparsers.add_parser("functions", help="query up to 20 known function entries")
    functions.add_argument("entries", nargs="+")
    functions.add_argument("--fields", default="entry_point,decompiled_code,callers,callees")
    xrefs = subparsers.add_parser("xrefs-to", help="query references to known addresses")
    xrefs.add_argument("addresses", nargs="+")
    args = parser.parse_args()
    if not args.url:
        parser.error("supply --url or GHIDRA_MCP_URL")
    if args.output.exists():
        parser.error(f"output already exists: {args.output}")
    query = {"program": args.program}
    if args.command == "functions":
        if len(args.entries) > 20:
            parser.error("the bridge accepts at most 20 function entries per query")
        endpoint = "/get_functions"
        query.update(functions=",".join(args.entries), fields=args.fields,
                     include_call_context="false")
    else:
        endpoint = "/get_xrefs_to"
        query.update(addresses=",".join(args.addresses), limit=0)
    # These two fixed GET endpoints do not change program data, annotations,
    # selection, project configuration, or the active GUI program.
    request = args.url.rstrip("/") + endpoint + "?" + urlencode(query)
    with urlopen(request, timeout=120) as response:
        evidence = json.load(response)
    with args.output.open("x", encoding="utf-8") as output:
        json.dump(evidence, output, indent=2)
        output.write("\n")
    print(f"saved {args.command} evidence to {args.output}")


if __name__ == "__main__":
    main()
