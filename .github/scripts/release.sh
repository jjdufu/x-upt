#!/usr/bin/env bash
# x-upt 发版脚本：按约定式提交计算下一个版本、改写版本文件、生成 CHANGELOG。
#
#   release.sh plan                 计算是否发版与版本号，写发版说明到 $NOTES_FILE
#   release.sh apply <X.Y.Z>        改写全部版本文件，并把 $NOTES_FILE 置顶写入 CHANGELOG.md
set -euo pipefail

ROOT="$(git rev-parse --show-toplevel)"
NOTES_FILE="${NOTES_FILE:-$(git rev-parse --absolute-git-dir)/release-notes.md}"
SEMVER_RE='^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$'
RELEASE_SUBJECT_RE='^chore\(release\):'
CC_RE='^([a-z]+)(\(([^)]*)\))?(!)?: (.+)$'

die() {
    echo "release.sh: $*" >&2
    exit 1
}

emit() {
    if [ -n "${GITHUB_OUTPUT:-}" ]; then
        printf '%s\n' "$@" >>"$GITHUB_OUTPUT"
    else
        printf '%s\n' "$@"
    fi
}

package_version() {
    awk '
        /^\[(workspace\.)?package\]/ { in_pkg = 1; next }
        /^\[/ { in_pkg = 0 }
        in_pkg && /^version = "/ { gsub(/^version = "|".*$/, ""); print; exit }
    ' "${ROOT}/Cargo.toml"
}

latest_tag() {
    git -C "$ROOT" describe --tags --abbrev=0 --match 'v[0-9]*.[0-9]*.[0-9]*' HEAD 2>/dev/null || true
}

commit_link() {
    local sha="$1" short="${1:0:7}"
    if [ -n "${GITHUB_REPOSITORY:-}" ]; then
        printf '[%s](%s/%s/commit/%s)' "$short" "${GITHUB_SERVER_URL:-https://github.com}" "$GITHUB_REPOSITORY" "$sha"
    else
        printf '%s' "$short"
    fi
}

classify() {
    local range="$1" rec sha subject body type scope bang desc item
    BREAKING=() FEATS=() FIXES=() PERFS=() REVERTS=()
    LEVEL=0
    while IFS= read -r -d $'\x1e' rec; do
        rec="${rec#$'\n'}"
        [ -n "$rec" ] || continue
        sha="${rec%%$'\x1f'*}"
        rec="${rec#*$'\x1f'}"
        subject="${rec%%$'\x1f'*}"
        body="${rec#*$'\x1f'}"
        [[ "$subject" =~ $RELEASE_SUBJECT_RE ]] && continue
        [[ "$subject" =~ $CC_RE ]] || continue
        type="${BASH_REMATCH[1]}"
        scope="${BASH_REMATCH[3]}"
        bang="${BASH_REMATCH[4]}"
        desc="${BASH_REMATCH[5]}"
        item="${desc} ($(commit_link "$sha"))"
        [ -n "$scope" ] && item="**${scope}:** ${item}"
        if [ -n "$bang" ] || grep -Eq '^BREAKING[ -]CHANGE: ' <<<"$body"; then
            BREAKING+=("$item")
            LEVEL=3
        fi
        case "$type" in
            feat)
                FEATS+=("$item")
                [ "$LEVEL" -ge 2 ] || LEVEL=2
                ;;
            fix | perf | revert)
                case "$type" in
                    fix) FIXES+=("$item") ;;
                    perf) PERFS+=("$item") ;;
                    revert) REVERTS+=("$item") ;;
                esac
                [ "$LEVEL" -ge 1 ] || LEVEL=1
                ;;
        esac
    done < <(git -C "$ROOT" log --no-merges --format='%H%x1f%s%x1f%b%x1e' "$range")
}

section() {
    local title="$1"
    shift
    [ "$#" -gt 0 ] || return 0
    printf '\n### %s\n\n' "$title"
    printf '* %s\n' "$@"
}

write_notes() {
    local version="$1" previous="$2" date compare
    date="$(TZ=Asia/Shanghai date +%F)"
    if [ -n "$previous" ] && [ -n "${GITHUB_REPOSITORY:-}" ]; then
        compare="${GITHUB_SERVER_URL:-https://github.com}/${GITHUB_REPOSITORY}/compare/${previous}...v${version}"
        printf '## [%s](%s) (%s)\n' "$version" "$compare" "$date"
    else
        printf '## %s (%s)\n' "$version" "$date"
    fi
    if [ -z "$previous" ]; then
        printf '\n首次发布。\n'
    fi
    section '⚠ BREAKING CHANGES' "${BREAKING[@]}"
    section 'Features' "${FEATS[@]}"
    section 'Bug Fixes' "${FIXES[@]}"
    section 'Performance Improvements' "${PERFS[@]}"
    section 'Reverts' "${REVERTS[@]}"
}

cmd_plan() {
    local previous base major minor patch version bump
    previous="$(latest_tag)"
    if [ -z "$previous" ]; then
        version="$(package_version)"
        [[ "$version" =~ $SEMVER_RE ]] || die "Cargo.toml 的版本不是 X.Y.Z：'${version}'"
        if git -C "$ROOT" rev-parse -q --verify "refs/tags/v${version}" >/dev/null; then
            die "tag v${version} 已存在但不是 HEAD 的祖先，拒绝发版"
        fi
        classify HEAD
        write_notes "$version" "" >"$NOTES_FILE"
        emit "release=true" "version=${version}" "previous=" "bump=initial"
        return 0
    fi

    base="${previous#v}"
    [[ "$base" =~ $SEMVER_RE ]] || die "上一个 tag 不是 vX.Y.Z：'${previous}'"
    major="${BASH_REMATCH[1]}" minor="${BASH_REMATCH[2]}" patch="${BASH_REMATCH[3]}"
    classify "${previous}..HEAD"

    bump="none"
    case "$LEVEL" in
        3) if [ "$major" -ge 1 ]; then bump="major"; else bump="minor"; fi ;;
        2) if [ "$major" -ge 1 ]; then bump="minor"; else bump="patch"; fi ;;
        1) bump="patch" ;;
    esac
    case "$bump" in
        major) version="$((major + 1)).0.0" ;;
        minor) version="${major}.$((minor + 1)).0" ;;
        patch) version="${major}.${minor}.$((patch + 1))" ;;
        none)
            emit "release=false" "version=${base}" "previous=${previous}" "bump=none"
            return 0
            ;;
    esac
    write_notes "$version" "$previous" >"$NOTES_FILE"
    emit "release=true" "version=${version}" "previous=${previous}" "bump=${bump}"
}

replace_in() {
    local file="$1" script="$2"
    sed -E -i "$script" "${ROOT}/${file}"
}

cmd_apply() {
    local version="${1:-}" tmp
    [[ "$version" =~ $SEMVER_RE ]] || die "apply 需要 X.Y.Z 版本号，得到 '${version}'"
    [ -s "$NOTES_FILE" ] || die "发版说明不存在：${NOTES_FILE}（先执行 plan）"

    replace_in Cargo.toml "/^\[(workspace\.)?package\]/,/^\[/ s/^version = \"[^\"]*\"/version = \"${version}\"/"
    replace_in Cargo.toml "/^\[workspace\.dependencies\]/,/^\[/ s/(x-upt(-[a-z]+)? = \{[^}]*version = \")[^\"]*(\")/\1${version}\3/"

    tmp="$(mktemp)"
    awk -v v="$version" '
        /^name = "x-upt(-[a-z]+)?"$/ { hit = 1; print; next }
        hit && /^version = "/ { print "version = \"" v "\""; hit = 0; next }
        { print }
    ' "${ROOT}/Cargo.lock" >"$tmp"
    cat "$tmp" >"${ROOT}/Cargo.lock"
    rm -f "$tmp"

    [ "$(package_version)" = "$version" ] || die "Cargo.toml 版本改写失败"
    [ "$(grep -A1 -E '^name = "x-upt"$' "${ROOT}/Cargo.lock" | grep -c "^version = \"${version}\"")" -eq 1 ] \
        || die "Cargo.lock 版本改写失败"

    if [ -f "${ROOT}/web/package.json" ]; then
        sed -E -i "0,/\"version\": \"[^\"]*\"/s//\"version\": \"${version}\"/" "${ROOT}/web/package.json"
    fi

    tmp="$(mktemp)"
    {
        printf '# Changelog\n\n'
        cat "$NOTES_FILE"
        if [ -f "${ROOT}/CHANGELOG.md" ]; then
            printf '\n'
            sed '1{/^# Changelog$/d}' "${ROOT}/CHANGELOG.md" | sed '/./,$!d'
        fi
    } >"$tmp"
    cat "$tmp" >"${ROOT}/CHANGELOG.md"
    rm -f "$tmp"
}

case "${1:-}" in
    plan) cmd_plan ;;
    apply) cmd_apply "${2:-}" ;;
    *) die "用法：release.sh plan | release.sh apply <X.Y.Z>" ;;
esac
