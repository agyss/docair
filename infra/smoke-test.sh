#!/usr/bin/env bash
# Smoke test for a running docair: the cluster from `terraform apply`
# (default http://localhost:8080) or a local docair-server (pass its URL).
#
#   infra/smoke-test.sh                       # after terraform -chdir=infra/terraform apply
#   infra/smoke-test.sh http://127.0.0.1:8081 # a local server with DOCAIR_STATIC_DIR
set -uo pipefail

BASE="${1:-http://localhost:8080}"
fails=0
pass() { echo "PASS  $*"; }
fail() { echo "FAIL  $*"; fails=$((fails + 1)); }

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

# 1. The frontend is served (Ingress / -> Service docair-web -> nginx).
if curl -sf "$BASE/" -o "$tmp/index.html" && grep -q "<app-root" "$tmp/index.html"; then
  pass "GET / serves the docair-web app"
else
  fail "GET / did not return the Angular app"
fi

# 2. An upload becomes a page (Ingress /api -> Service docair-server -> pod).
mkdir -p "$tmp/demo/src"
printf '%s\n' '/*** The demo API. @http POST /api/orders ***/' 'pub fn create() {}' > "$tmp/demo/src/main.rs"
printf '# Demo\n' > "$tmp/demo/README.md"
code=$(curl -s -o "$tmp/scan.json" -w '%{http_code}' "$BASE/api/scan" \
  -F "file=@$tmp/demo/src/main.rs;filename=demo/src/main.rs" \
  -F "file=@$tmp/demo/README.md;filename=demo/README.md")
# The page drawn by the CLI holds its pictures packed (docair-packed).
if [ "$code" = 200 ] && jq -e '.system == "demo" and .nodes >= 2 and (.html | contains("docair-packed"))' "$tmp/scan.json" >/dev/null; then
  pass "POST /api/scan turns an upload into a page ($(jq -r '"\(.nodes) nodes, \(.edges) edges"' "$tmp/scan.json"))"
else
  fail "POST /api/scan answered $code: $(head -c 200 "$tmp/scan.json")"
fi

# 3. Paths that leave the upload are refused.
code=$(curl -s -o /dev/null -w '%{http_code}' "$BASE/api/scan" -F "file=@$tmp/demo/README.md;filename=../escape.md")
[ "$code" = 400 ] && pass "an upload path with .. is refused (400)" || fail "an upload path with .. answered $code, expected 400"

# 4. In the cluster: every pod in the namespace docair is ready.
if command -v kubectl >/dev/null && kubectl get ns docair >/dev/null 2>&1; then
  if kubectl wait --for=condition=Ready pods --all -n docair --timeout=60s >/dev/null; then
    pass "all pods in namespace docair are Ready ($(kubectl get pods -n docair --no-headers | wc -l) pods)"
  else
    fail "pods in namespace docair are not Ready: $(kubectl get pods -n docair --no-headers 2>&1 | tr '\n' ';')"
  fi
else
  echo "SKIP  kubectl checks (no cluster context: terraform -chdir=infra/terraform output -raw kubeconfig > ~/.kube/config)"
fi

echo
[ "$fails" -eq 0 ] && echo "smoke test passed: $BASE" || echo "smoke test failed: $fails check(s)"
exit "$fails"
