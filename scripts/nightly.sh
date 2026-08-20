#!/bin/sh
# scripts/nightly.sh — 本地每日构建并发布 nightly 到 GitHub Releases
#
# 行为：
#   - 拉取最新 main；若 HEAD 与上次 nightly 的 commit 相同（代码无更新）则跳过
#   - 构建前端 + 后端 release，上传为 prerelease "nightly"
#   - 固定 tag=nightly，每次覆盖（先删后建），文件名带日期+commit 便于识别
#   - 任意步骤失败即中止：不会覆盖上次成功的 nightly（安全特性）
#
# 前提（首次手动执行一次）：
#   pkg install -y gh
#   gh auth login            # 选 GitHub.com → HTTPS → 浏览器/token
#
# cron 用法见文末注释。

set -eu

# cron 的默认 PATH 只有 /usr/bin:/bin，必须显式补全，否则 cargo/npm/gh 全找不到。
export PATH="/usr/local/bin:/usr/local/sbin:/usr/bin:/bin:/usr/sbin:/sbin:${HOME}/.cargo/bin"

REPO="/root/git-repos/freebsd-web-panel"
BRANCH="main"

cd "$REPO"

echo "=== $(date) nightly build start ==="

# 1) fetch 只下载远程对象，绝不改工作区——永远安全。
git fetch origin "$BRANCH"

# 2) fast-forward 到最新（构建 Jail 是专用检出，无本地改动需保护）。
git pull --ff-only

# 1.5) 代码无更新则跳过：对比当前 HEAD 与上次 nightly 发布的 commit。
#      状态存于 GitHub release 自身（targetCommitish），不依赖本地 marker 文件。
CURRENT_SHA="$(git rev-parse HEAD)"
LAST_NIGHTLY_SHA="$(gh release view nightly --json targetCommitish -q .targetCommitish 2>/dev/null || true)"
if [ "$LAST_NIGHTLY_SHA" = "$CURRENT_SHA" ]; then
    echo "=== 代码无更新（HEAD ${CURRENT_SHA} 与上次 nightly 相同），跳过发布 ==="
    exit 0
fi

SHORT_SHA="$(git rev-parse --short HEAD)"
DATE="$(date +%Y%m%d)"
ASSET="target/fwp-nightly-${DATE}-${SHORT_SHA}-freebsd-amd64"

echo "--- commit ${SHORT_SHA} (${DATE}) ---"

# 生成自上次 nightly 以来的全部 commit 列表（差异间所有提交，非仅最新一条）。
# 边界兜底：首次发布（LAST_NIGHTLY_SHA 为空）或该 commit 本地不存在（历史被改写）
# 时退化为仅最新一条。
if [ -n "$LAST_NIGHTLY_SHA" ] && git rev-parse -q --verify "${LAST_NIGHTLY_SHA}^{commit}" >/dev/null 2>&1; then
    CHANGELOG="$(git log --format='- %h %s' "${LAST_NIGHTLY_SHA}..${CURRENT_SHA}")"
else
    CHANGELOG="$(git log -1 --format='- %h %s')"
fi

# 2) 前端：输出到 web/，供后端默认 embed-web feature 内嵌
cd frontend
npm ci
npm run build
cd ..

# 3) 后端 release 构建（增量；首次较慢，后续秒级）
cargo build --release

# 4) 重命名产物，带上日期与 commit 便于识别
cp "target/release/fwp" "$ASSET"

# 5) 覆盖 GitHub 的 nightly release（固定 tag，每次重建）
#    --prerelease 避免盖过正式版 latest；--cleanup-tag 删除旧 tag
gh release delete nightly --cleanup-tag --yes 2>/dev/null || true

gh release create nightly "$ASSET" \
    --target "$CURRENT_SHA" \
    --prerelease \
    --title "Nightly ${DATE}" \
    --notes "Automated nightly build of \`${SHORT_SHA}\`.

- Branch: ${BRANCH}
- Commit: $(git rev-parse HEAD)
- Built:  $(date)
- Target: FreeBSD 15.x amd64 (single binary, web assets embedded)

## Changes since last nightly

${CHANGELOG}"

# 清理本次临时产物（target/release/fwp 保留供本地使用）
rm -f "$ASSET"

echo "=== $(date) nightly published: ${DATE} ${SHORT_SHA} ==="

# ── cron 配置 ──────────────────────────────────────────────────────
# 以 root 执行：  crontab -e
# 加入下面一行（每天凌晨 3:00 构建，日志写到 /var/log/fwp-nightly.log）：
#
#   0 3 * * * /root/git-repos/freebsd-web-panel/scripts/nightly.sh >> /var/log/fwp-nightly.log 2>&1
#
# 失败排查： tail -50 /var/log/fwp-nightly.log
# 重要：构建失败时不会触碰 GitHub —— 上一次成功的 nightly 保留，用户始终能下到可用版本。
