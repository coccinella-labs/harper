#!/bin/bash

set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
DIM='\033[2m'
BOLD='\033[1m'
NC='\033[0m'

ORANGE='\033[38;2;255;122;36m'

TOTAL_STEPS=14
PASSED_STEPS=0
WARNED_STEPS=0
CURRENT_STEP=""

CI_MODE=false

print_banner() {
    echo

    echo -e "${ORANGE}   ⣀⠴⠶⠛⢻⣄${NC}"
    echo -e "${ORANGE}⠰⠛⠛⠙⢷⣦⡀ ⠹⣧⡀${NC}"
    echo -e "${ORANGE}     ⠈⠻⣦⡀⢈⣷⣄⣀⡀${NC}"
    echo -e "${ORANGE}      ⢀⣼⣿⣿⣿⣿⣿⣿⣦${NC}"
    echo -e "${ORANGE}     ⢠⣿⣿⣅⣼⣿⣿⣿⣿⣿⢇⣴⣾⣿⣿⣷⣦⣄    ⢀⣀⣀⣀⣀${NC}"
    echo -e "${ORANGE}     ⢸⣿⣿⣿⣿⣿⣿⣿⠿⠋⢸⣿⣿⣿⣿⣿⣿⣿⣷⡀⢠⣾⣿⣿⣿⣿⣿⣿⣦⡀${NC}"
    echo -e "${ORANGE}     ⠈⠛⠻⠟⠛⠛⢉⣴⢶⣤⣈⠻⣿⣿⣿⣿⢟⡛⢿⡇⣿⡿⢟⠛⢿⣿⣿⣿⣿⣿⣦${NC}"
    echo -e "${ORANGE}          ⣠⡿⠁⢀⣤⣭⣥⣬⣿⠟⠁⣾⣷ ⣠⣤⡶⣿⣿⣌⠻⣿⣿⣿⣿⣿⣧${NC}"
    echo -e "${ORANGE}         ⣴⠟ ⢀⣿⠋⠉⠉⠉⠁   ⢹⣧⠉⠁ ⠈⠻⢿⣦⡙⣿⣿⣿⣿⡿${NC}"
    echo -e "${ORANGE}       ⢠⡾⠃  ⣾⠃         ⢻⡆     ⠘⢷⣌⠉⠉⠁${NC}"
    echo -e "${ORANGE}      ⣰⡟⠁  ⠸⠏          ⠈⢿⠄      ⠻⣦${NC}"

    echo
    echo -e "${BLUE}╭──────────────────────────────────────────────╮${NC}"
    echo -e "${BLUE}│          Harper Validation Suite             │${NC}"
    echo -e "${BLUE}│        Build. Lint. Test. Verify.            │${NC}"
    echo -e "${BLUE}╰──────────────────────────────────────────────╯${NC}"

    echo
}

print_status() {
    local status=$1
    local message=$2

    case $status in
        PASS)
            echo -e "${GREEN}PASS${NC} $message"
            ;;
        FAIL)
            echo -e "${RED}FAIL${NC} $message"
            ;;
        WARN)
            echo -e "${YELLOW}WARN${NC} $message"
            ;;
        INFO)
            echo -e "${BLUE}INFO${NC} $message"
            ;;
    esac
}

draw_progress_bar() {
    local current=$1
    local total=$2
    local width=28

    local percent=$((current * 100 / total))
    local filled=$((current * width / total))
    local empty=$((width - filled))

    printf "${BLUE}["

    if [ "$filled" -gt 0 ]; then
        printf "%0.s█" $(seq 1 "$filled")
    fi

    if [ "$empty" -gt 0 ]; then
        printf "%0.s░" $(seq 1 "$empty")
    fi

    printf "]${NC}"

    printf " ${BOLD}%3d%%${NC}" "$percent"
}

run_command_with_log() {
    local log_file
    log_file=$(mktemp)

    local spinner='|/-\'
    local delay=0.08
    local pid
    local start_time

    start_time=$(date +%s)

    "$@" >"$log_file" 2>&1 &
    pid=$!

    tput civis 2>/dev/null || true

    while kill -0 "$pid" 2>/dev/null; do
        for ((i = 0; i < ${#spinner}; i++)); do
            kill -0 "$pid" 2>/dev/null || break

            local elapsed
            elapsed=$(( $(date +%s) - start_time ))

            printf "\r\033[K"

            draw_progress_bar "$PASSED_STEPS" "$TOTAL_STEPS"

            printf " ${ORANGE}%s${NC} ${BOLD}%s${NC} ${DIM}(%ss)${NC}" \
                "${spinner:i:1}" \
                "$CURRENT_STEP" \
                "$elapsed"

            sleep "$delay"
        done
    done

    local exit_code=0
    wait "$pid" || exit_code=$?

    printf "\r\033[K"

    tput cnorm 2>/dev/null || true

    if [ "$exit_code" -eq 0 ]; then
        rm -f "$log_file"
        return 0
    fi

    echo
    echo -e "${RED}╭─ command output ─────────────────────────────${NC}"
    cat "$log_file"
    echo -e "${RED}╰──────────────────────────────────────────────${NC}"

    rm -f "$log_file"

    return 1
}

run_required_step() {
    local step=$1
    local title=$2
    local fail_message=$3
    local tip_message=$4

    shift 4

    CURRENT_STEP="$title"

    echo
    print_status "INFO" "[$step/$TOTAL_STEPS] $title"

    if run_command_with_log "$@"; then
        PASSED_STEPS=$((PASSED_STEPS + 1))
        print_status "PASS" "$title"
    else
        print_status "FAIL" "$fail_message"

        if [ -n "$tip_message" ]; then
            echo -e "${DIM}$tip_message${NC}"
        fi

        exit 1
    fi
}

run_optional_step() {
    local step=$1
    local title=$2

    shift 2

    CURRENT_STEP="$title"

    echo
    print_status "INFO" "[$step/$TOTAL_STEPS] $title"

    if run_command_with_log "$@"; then
        PASSED_STEPS=$((PASSED_STEPS + 1))
        print_status "PASS" "$title"
    else
        WARNED_STEPS=$((WARNED_STEPS + 1))
        print_status "WARN" "$title"
    fi
}

check_project_root() {
    if [ ! -f "Cargo.toml" ]; then
        print_status "FAIL" "Not in Harper project root (Cargo.toml not found)"
        exit 1
    fi
}

check_large_files() {
    local large_files

    large_files=$(find . -type f -size +10M \
        -not -path "./target/*" \
        -not -path "./.git/*" 2>/dev/null || true)

    if [ -n "$large_files" ]; then
        print_status "FAIL" "Large files detected (>10MB)"
        echo "$large_files"
        return 1
    fi

    return 0
}

check_sensitive_files() {
    local sensitive_files

    sensitive_files=$(find . \
        \( -name "*.key" -o -name "*.pem" -o -name "*.env" \) \
        -not -path "./target/*" \
        -not -path "./.git/*" 2>/dev/null || true)

    if [ -n "$sensitive_files" ]; then
        print_status "WARN" "Potential sensitive files found"
        echo "$sensitive_files"
        WARNED_STEPS=$((WARNED_STEPS + 1))
    else
        print_status "PASS" "No sensitive files found in repository"
        PASSED_STEPS=$((PASSED_STEPS + 1))
    fi

    return 0
}

check_yaml() {
    if ! command -v yamllint >/dev/null 2>&1; then
        print_status "WARN" "yamllint not installed (run: pip install yamllint)"
        WARNED_STEPS=$((WARNED_STEPS + 1))
        return 0
    fi

    local yaml_files

    yaml_files=$(find . \
        \( -name "*.yml" -o -name "*.yaml" \) \
        -not -path "./target/*" \
        -not -path "./.git/*" 2>/dev/null || true)

    if [ -z "$yaml_files" ]; then
        print_status "PASS" "No YAML files to validate"
        PASSED_STEPS=$((PASSED_STEPS + 1))
        return 0
    fi

    if echo "$yaml_files" | xargs yamllint >/dev/null 2>&1; then
        print_status "PASS" "YAML files are valid"
        PASSED_STEPS=$((PASSED_STEPS + 1))
        return 0
    fi

    print_status "FAIL" "YAML validation failed"
    echo "Run 'yamllint .github/workflows/*.yml docker/docker-compose.yml' for details"

    return 1
}

check_todos() {
    local todo_count

    todo_count=$(
        rg --glob '*.rs' --count-matches \
            '(//|///|/\*|\*|//!).*(TODO|FIXME|XXX)' \
            . 2>/dev/null || true
    )

    todo_count=$(
        printf '%s\n' "$todo_count" \
            | awk -F: '{sum += $NF} END {print sum + 0}'
    )

    if [ "$todo_count" -eq 0 ]; then
        print_status "PASS" "No unresolved TODO comments found"
        PASSED_STEPS=$((PASSED_STEPS + 1))
    else
        print_status "WARN" "Found $todo_count unresolved TODO/FIXME/XXX comments"
        echo "Consider resolving these or documenting why they're needed"
        WARNED_STEPS=$((WARNED_STEPS + 1))
    fi

    return 0
}

check_cargo_audit() {
    if ! command -v cargo-audit >/dev/null 2>&1 && \
       [ ! -x "$HOME/.cargo/bin/cargo-audit" ]; then
        print_status "WARN" \
            "cargo-audit not installed (run: cargo install cargo-audit)"
        WARNED_STEPS=$((WARNED_STEPS + 1))
        return 0
    fi

    local audit_bin="cargo-audit"

    if ! command -v cargo-audit >/dev/null 2>&1; then
        audit_bin="$HOME/.cargo/bin/cargo-audit"
    fi

    CURRENT_STEP="Security audit"

    if run_command_with_log "$audit_bin" audit --quiet; then
        print_status "PASS" "Security audit passed"
        PASSED_STEPS=$((PASSED_STEPS + 1))
        return 0
    fi

    print_status "FAIL" "Security audit found vulnerabilities"
    echo "Run '$audit_bin audit' for details"

    return 1
}

check_cargo_deny() {
    if ! command -v cargo-deny >/dev/null 2>&1 && \
       [ ! -x "$HOME/.cargo/bin/cargo-deny" ]; then
        print_status "WARN" \
            "cargo-deny not installed (run: cargo install cargo-deny)"
        WARNED_STEPS=$((WARNED_STEPS + 1))
        return 0
    fi

    local deny_bin="cargo-deny"

    if ! command -v cargo-deny >/dev/null 2>&1; then
        deny_bin="$HOME/.cargo/bin/cargo-deny"
    fi

    CURRENT_STEP="cargo-deny policy check"

    if run_command_with_log "$deny_bin" check; then
        print_status "PASS" "cargo-deny policy checks passed"
        PASSED_STEPS=$((PASSED_STEPS + 1))
        return 0
    fi

    print_status "FAIL" "cargo-deny reported dependency policy issues"
    echo "Run '$deny_bin check' for details"

    return 1
}

check_benchmarks() {
    cargo bench --workspace --quiet >/dev/null 2>&1
}

check_integration_tests() {
    cargo test --tests --workspace --quiet >/dev/null 2>&1
}

release_binary_size() {
    if ! command -v stat >/dev/null 2>&1; then
        echo "unknown"
        return 0
    fi

    local bin

    bin=$(
        cargo metadata --no-deps --format-version 1 2>/dev/null \
        | jq -r '.packages[0].targets[] | select(.kind[]=="bin") | .name' \
        2>/dev/null || echo "harper"
    )

    stat -f%z "target/release/$bin" 2>/dev/null \
        || stat -c%s "target/release/$bin" 2>/dev/null \
        || echo "unknown"
}

print_summary() {
    echo
    echo -e "${GREEN}╭──────────────────────────────────────────────╮${NC}"
    echo -e "${GREEN}│           Validation Complete ✓              │${NC}"
    echo -e "${GREEN}╰──────────────────────────────────────────────╯${NC}"

    echo
    echo -e "${BOLD}Summary${NC}"
    echo " • Passed steps : $PASSED_STEPS/$TOTAL_STEPS"
    echo " • Warning steps: $WARNED_STEPS"
    echo " • Required checks: all passed"

    echo
}

main() {
    check_project_root

    if [ "${1:-}" = "--ci" ]; then
        CI_MODE=true
        TOTAL_STEPS=7
    else
        TOTAL_STEPS=14
    fi

    print_banner

    print_status "INFO" "Starting validation checks..."

    local _step=0

    if [ "$CI_MODE" = false ]; then
        _step=$((_step + 1))
        run_required_step "$_step" "Rust compilation check" \
            "Rust compilation failed" \
            "Run 'cargo check --workspace' for details" \
            cargo check --workspace --quiet
    fi

    if [ "$CI_MODE" = false ]; then
        _step=$((_step + 1))
        run_required_step "$_step" "Clippy linter" \
            "Clippy reported issues that need attention" \
            "Run 'cargo clippy --all-targets --all-features --workspace -- -A clippy::pedantic -D warnings' for details" \
            cargo clippy --all-targets --all-features --workspace \
            --quiet -- -A clippy::pedantic -D warnings
    fi

    _step=$((_step + 1))
    run_required_step "$_step" "Formatting check" \
        "Code formatting issues found" \
        "Run 'cargo fmt --all' to fix formatting" \
        cargo fmt --all -- --check

    if [ "$CI_MODE" = false ]; then
        _step=$((_step + 1))
        run_required_step "$_step" "Workspace tests" \
            "Workspace tests failed" \
            "Run 'cargo test --workspace' for details" \
            cargo test --workspace --quiet
    fi

    _step=$((_step + 1))
    echo
    print_status "INFO" "[${_step}/$TOTAL_STEPS] Security audit"

    if ! check_cargo_audit; then
        exit 1
    fi

    _step=$((_step + 1))
    echo
    print_status "INFO" "[${_step}/$TOTAL_STEPS] Large file check"

    if check_large_files; then
        print_status "PASS" "No large files found"
        PASSED_STEPS=$((PASSED_STEPS + 1))
    else
        exit 1
    fi

    _step=$((_step + 1))
    echo
    print_status "INFO" "[${_step}/$TOTAL_STEPS] Sensitive file check"
    check_sensitive_files

    _step=$((_step + 1))
    echo
    print_status "INFO" "[${_step}/$TOTAL_STEPS] YAML validation"

    if ! check_yaml; then
        exit 1
    fi

    _step=$((_step + 1))
    echo
    print_status "INFO" "[${_step}/$TOTAL_STEPS] TODO/FIXME/XXX scan"
    check_todos

    if [ "$CI_MODE" = false ]; then
        _step=$((_step + 1))
        run_required_step "$_step" "Documentation build" \
            "Documentation build failed" \
            "Run 'cargo doc --no-deps --workspace' for details" \
            cargo doc --no-deps --workspace --quiet
    fi

    _step=$((_step + 1))
    echo
    print_status "INFO" "[${_step}/$TOTAL_STEPS] cargo-deny policy check"

    if ! check_cargo_deny; then
        exit 1
    fi

    if [ "$CI_MODE" = false ]; then
        _step=$((_step + 1))
        run_optional_step "$_step" "Workspace benchmarks" \
            check_benchmarks

        _step=$((_step + 1))
        run_optional_step "$_step" "Integration tests" \
            check_integration_tests

        _step=$((_step + 1))
        run_required_step "$_step" "Release build" \
            "Release build failed" \
            "Run 'cargo build --release --workspace' for details" \
            cargo build --release --workspace --quiet

        print_status \
            "PASS" \
            "Release build successful (binary size: $(release_binary_size) bytes)"
    fi

    print_summary
}

main "$@"
