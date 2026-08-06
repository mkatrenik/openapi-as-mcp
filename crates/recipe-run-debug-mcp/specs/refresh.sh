#!/usr/bin/env bash
# Re-fetch the vendored OpenAPI specs, then regenerate the models.
#
# The specs are served unauthenticated over **https** (note: the UI's own generate-schemas.sh uses
# http:// for dev, which does not answer). Review the spec diff before committing — it is the
# clearest signal of a backend change, and `cargo test` will flag anything that breaks parsing.
set -euo pipefail

cd "$(dirname "$0")/.."

ENV="${1:-dev}"
case "$ENV" in
  dev)
    AGGREGATOR="https://reciperun-aggregator.k8s.euw1.dev.gcp.ivxs.uk"
    RECIPE_STORE="https://recipe-store.k8s.euw1.dev.gcp.ivxs.uk"
    GATEWAY="https://api-gateway-service.k8s.euw1.dev.gcp.ivxs.uk"
    ;;
  stg)
    AGGREGATOR="https://reciperun-aggregator-stg.k8s.euw1.dev.gcp.ivxs.uk"
    RECIPE_STORE="https://recipe-store-stg.k8s.euw1.dev.gcp.ivxs.uk"
    GATEWAY="https://api-gateway-service-stg.k8s.euw1.dev.gcp.ivxs.uk"
    ;;
  prod)
    AGGREGATOR="https://reciperun-aggregator-hp.k8s.euw1.prod.gcp.ivxs.uk"
    RECIPE_STORE="https://recipe-store.k8s.euw1.prod.gcp.ivxs.uk"
    GATEWAY="https://api-gateway-service.k8s.euw1.prod.gcp.ivxs.uk"
    ;;
  *)
    echo "unknown env: $ENV (use dev, stg or prod)" >&2
    exit 1
    ;;
esac

fetch() {
  local name="$1" base="$2"
  echo "fetching ${name} from ${base}/openapi.json"
  curl --fail --silent --show-error --max-time 30 "${base}/openapi.json" \
    | python3 -m json.tool > "specs/${name}.openapi.json"
}

fetch reciperun-aggregator "$AGGREGATOR"
fetch recipe-store "$RECIPE_STORE"
fetch api-gateway "$GATEWAY"

echo
echo "regenerating models"
cargo run --quiet --example generate_models

echo
echo "done — review 'git diff specs/ src/api/generated.rs', then run: cargo test"
