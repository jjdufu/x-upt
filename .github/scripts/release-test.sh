#!/usr/bin/env bash
# release.sh 的自测：在临时 git 仓库里构造各种提交组合，核对版本计算与文件改写。
set -euo pipefail

unset GITHUB_OUTPUT GITHUB_REPOSITORY GITHUB_SERVER_URL NOTES_FILE

SRC="$(git rev-parse --show-toplevel)"
SCRIPT="${SRC}/.github/scripts/release.sh"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
FAILS=0
CASES=0

new_repo() {
    local dir="${WORK}/$1" f
    mkdir -p "$dir"
    for f in Cargo.toml Cargo.lock; do
        cp "${SRC}/${f}" "${dir}/${f}"
    done
    (
        cd "$dir"
        "$SCRIPT_APPLY_FIXTURE" 0.1.0
        git init -q -b main
        git config user.name test
        git config user.email test@example.invalid
        git add -A
        git commit -qm "基线提交"
    )
    printf '%s' "$dir"
}

commit() {
    local dir="$1" subject="$2" body="${3:-}"
    if [ -n "$body" ]; then
        git -C "$dir" commit -q --allow-empty -m "$subject" -m "$body"
    else
        git -C "$dir" commit -q --allow-empty -m "$subject"
    fi
}

plan() {
    local dir="$1"
    (cd "$dir" && "$SCRIPT" plan)
}

expect() {
    local name="$1" dir="$2" want="$3" got
    CASES=$((CASES + 1))
    got="$(plan "$dir" | grep -E '^(release|version|bump)=' | paste -sd' ')"
    if [ "$got" = "$want" ]; then
        echo "ok   ${name}: ${got}"
    else
        echo "FAIL ${name}: want '${want}' got '${got}'"
        FAILS=$((FAILS + 1))
    fi
}

check() {
    local name="$1"
    shift
    CASES=$((CASES + 1))
    if "$@"; then
        echo "ok   ${name}"
    else
        echo "FAIL ${name}"
        FAILS=$((FAILS + 1))
    fi
}

SCRIPT_APPLY_FIXTURE="${WORK}/fixture.sh"
cat >"$SCRIPT_APPLY_FIXTURE" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
v="$1"
sed -E -i "/^\[(workspace\.)?package\]/,/^\[/ s/^version = \"[^\"]*\"/version = \"${v}\"/" Cargo.toml
sed -E -i "/^\[workspace\.dependencies\]/,/^\[/ s/(x-upt(-[a-z]+)? = \{[^}]*version = \")[^\"]*(\")/\1${v}\3/" Cargo.toml
awk -v v="$v" '/^name = "x-upt(-[a-z]+)?"$/{h=1;print;next} h&&/^version = "/{print "version = \"" v "\"";h=0;next}{print}' Cargo.lock >Cargo.lock.new
mv Cargo.lock.new Cargo.lock
SH
chmod +x "$SCRIPT_APPLY_FIXTURE"

r="$(new_repo initial)"
commit "$r" "非约定式提交"
expect "无 tag 首次发版" "$r" "release=true version=0.1.0 bump=initial"
(cd "$r" && "$SCRIPT" apply 0.1.0)
check "首次发版 CHANGELOG 含首次发布" grep -q '^首次发布。$' "${r}/CHANGELOG.md"
check "首次发版 CHANGELOG 标题" grep -q '^## 0.1.0 (' "${r}/CHANGELOG.md"
git -C "$r" add -A && git -C "$r" commit -qm "chore(release): v0.1.0 [skip ci]"
git -C "$r" tag -a v0.1.0 -m v0.1.0

expect "tag 后无新提交" "$r" "release=false version=0.1.0 bump=none"
commit "$r" "docs: 补充说明"
commit "$r" "chore(deps): 升级依赖"
commit "$r" "ci: 调整缓存"
commit "$r" "refactor(front): 整理路由"
commit "$r" "Feat: 大写类型不算"
commit "$r" "feat 缺少冒号不算"
expect "只有 docs/chore/ci/refactor" "$r" "release=false version=0.1.0 bump=none"

commit "$r" "fix: 修复日志脱敏"
expect "0.x fix" "$r" "release=true version=0.1.1 bump=patch"
commit "$r" "feat(grpc): 启用描述集导出"
expect "0.x feat" "$r" "release=true version=0.1.1 bump=patch"
r2="$(new_repo pre-major-breaking)"
git -C "$r2" tag -a v0.1.0 -m v0.1.0
commit "$r2" "feat!: 调整接口契约"
expect "0.x feat!" "$r2" "release=true version=0.2.0 bump=minor"
r3="$(new_repo pre-major-footer)"
git -C "$r3" tag -a v0.1.0 -m v0.1.0
commit "$r3" "refactor: 改写存储模型" "BREAKING CHANGE: 存储格式升级"
expect "0.x BREAKING CHANGE 脚注" "$r3" "release=true version=0.2.0 bump=minor"
r4="$(new_repo perf)"
git -C "$r4" tag -a v0.1.0 -m v0.1.0
commit "$r4" "perf(core): 减少分配"
expect "perf → patch" "$r4" "release=true version=0.1.1 bump=patch"

r5="$(new_repo post-major)"
git -C "$r5" tag -a v1.2.3 -m v1.2.3
commit "$r5" "fix: a"
expect "1.x fix" "$r5" "release=true version=1.2.4 bump=patch"
commit "$r5" "feat: b"
expect "1.x feat" "$r5" "release=true version=1.3.0 bump=minor"
commit "$r5" "fix(api)!: c"
expect "1.x fix!" "$r5" "release=true version=2.0.0 bump=major"

(cd "$r" && "$SCRIPT" plan >/dev/null && "$SCRIPT" apply 0.1.1)
check "Cargo.toml 版本" grep -q '^version = "0.1.1"$' "${r}/Cargo.toml"
check "Cargo.lock x-upt 版本" test "$(grep -A1 -E '^name = "x-upt"$' "${r}/Cargo.lock" | grep -c '^version = "0.1.1"')" -eq 1
check "CHANGELOG 新段落在前" test "$(grep -m1 '^## ' "${r}/CHANGELOG.md" | cut -c1-9)" = "## 0.1.1 "
check "CHANGELOG 保留旧段落" grep -q '^## 0.1.0 (' "${r}/CHANGELOG.md"
check "CHANGELOG 只有一个标题" test "$(grep -c '^# Changelog$' "${r}/CHANGELOG.md")" -eq 1
check "CHANGELOG Features 带 scope" grep -q '^\* \*\*grpc:\*\* 启用描述集导出 (' "${r}/CHANGELOG.md"
check "CHANGELOG Bug Fixes" grep -q '^\* 修复日志脱敏 (' "${r}/CHANGELOG.md"
check "CHANGELOG 不收 docs/refactor" bash -c "! grep -q '整理路由\|补充说明' '${r}/CHANGELOG.md'"
check "非法版本被拒绝" bash -c "cd '$r' && ! '$SCRIPT' apply 1.2 2>/dev/null"

r6="$(new_repo linked)"
git -C "$r6" tag -a v0.1.0 -m v0.1.0
commit "$r6" "fix: 链接格式"
(cd "$r6" && GITHUB_REPOSITORY=jjdufu/x-upt "$SCRIPT" plan >/dev/null)
check "Actions 下标题带 compare 链接" grep -q '^## \[0.1.1\](https://github.com/jjdufu/x-upt/compare/v0.1.0...v0.1.1) (' "${r6}/.git/release-notes.md"
check "Actions 下条目带提交链接" grep -q '^\* 链接格式 (\[[0-9a-f]\{7\}\](https://github.com/jjdufu/x-upt/commit/[0-9a-f]\{40\}))$' "${r6}/.git/release-notes.md"
(cd "$r6" && out="$(mktemp)" && GITHUB_OUTPUT="$out" "$SCRIPT" plan >/dev/null && grep -qx 'version=0.1.1' "$out" && rm -f "$out")
check "GITHUB_OUTPUT 写入结果" test "$?" -eq 0

git -C "$r" add -A && git -C "$r" commit -qm "chore(release): v0.1.1 [skip ci]"
git -C "$r" tag -a v0.1.1 -m v0.1.1
expect "release 提交之后" "$r" "release=false version=0.1.1 bump=none"

echo "${CASES} cases, ${FAILS} failed"
[ "$FAILS" -eq 0 ]
