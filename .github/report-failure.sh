#!/usr/bin/env bash
# Actions logs need a login even on a public repo; annotations don't, so put
# the interesting tail of a failed build log there.
f=${1:-build.log}
[ -f "$f" ] || exit 0
msg=$(grep -E "error|Error|ERROR|FAILED|What went wrong|^> |cannot find|undefined" "$f" | grep -v "^warning" | tail -40)
[ -n "$msg" ] || msg=$(tail -40 "$f")
msg=${msg//'%'/'%25'}
msg=${msg//$'\r'/}
msg=${msg//$'\n'/'%0A'}
echo "::error title=$f::$msg"
