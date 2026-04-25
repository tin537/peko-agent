#!/usr/bin/env bash
# Run every phase device-test in sequence, print a summary.
# Stops on first failure but prints which phase failed.

set -uo pipefail
REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
LOG_DIR="$REPO_ROOT/tests/device-test/logs"
mkdir -p "$LOG_DIR"
TIMESTAMP=$(date -u +%Y%m%dT%H%M%SZ)
SUMMARY=()

PHASES=(1 2 3 4)

printf "\n\033[1;36m=== Peko Agent on-device test suite (%s) ===\033[0m\n" "$TIMESTAMP"

for n in "${PHASES[@]}"; do
    LOG_FILE="$LOG_DIR/phase${n}_${TIMESTAMP}.log"
    printf "\n\033[1;36m=== PHASE %s ===\033[0m  (log: %s)\n" "$n" "$LOG_FILE"
    if bash "$REPO_ROOT/tests/device-test/phase${n}.sh" 2>&1 | tee "$LOG_FILE"; then
        SUMMARY+=("phase${n}=PASS")
    else
        SUMMARY+=("phase${n}=FAIL")
        printf "\n\033[1;31m=== PHASE %s FAILED — stopping suite ===\033[0m\n" "$n"
        break
    fi
done

printf "\n\033[1;36m=== Summary ===\033[0m\n"
for s in "${SUMMARY[@]}"; do
    case "$s" in
        *=PASS) printf "  \033[1;32m✓\033[0m %s\n" "$s" ;;
        *=FAIL) printf "  \033[1;31m✗\033[0m %s\n" "$s" ;;
    esac
done

# Exit code: 0 only if every phase passed.
for s in "${SUMMARY[@]}"; do
    [[ "$s" == *=FAIL ]] && exit 1
done
[[ ${#SUMMARY[@]} -eq ${#PHASES[@]} ]] || exit 1
exit 0
