#!/usr/bin/env bash
# upstream-sync.sh — 将上游 clash-verge-rev 最新 release tag 合并到新的 fa/<tag>-fa.0 分支
#
# 用法:
#   ./.claude/skills/upstream-sync/upstream-sync.sh              # 自动取上游最新 release tag
#   ./.claude/skills/upstream-sync/upstream-sync.sh v2.4.8       # 指定目标 tag
#   ./.claude/skills/upstream-sync/upstream-sync.sh --check-only # 仅预检，不合并
#   ./.claude/skills/upstream-sync/upstream-sync.sh --no-verify  # 合并但跳过编译验证（留给 skill Step 4）
#
# 退出码:
#   0  成功（无残余冲突；若未跳过验证则 typecheck + cargo check 通过）
#   1  参数或环境错误（缺依赖 / 未完成的 merge / 已是最新）
#   2  存在需要 AI 智能合并的冲突（输出冲突文件列表 + AI_HINTS 到 stdout）
#   3  验证失败（已合并但 typecheck / cargo check 不通过）
#
# 与 .claude/skills/upstream-sync/SKILL.md 是协作关系:
#   - 脚本: 机械可重复操作（推导 tag、建分支、git merge、固定策略 --ours、emit AI_HINTS、快速验证）
#   - skill+AI: 需要语义理解的操作（// FORK: 标记复审、智能三向合并、i18n 重新生成、连带影响排查、汇报）
#
# clash-verge-rev fork 合并逻辑要点（详见 SKILL.md 与 .github/CONFLICT_RESOLUTION_GUIDE.md）:
#   - fork 改动用 `// FORK:` 注释标记 —— 这是识别"必须保留"代码的首要依据
#   - 脚本只对纯 fork 所有的 Changelog.md 自动 --ours；其余冲突一律交 AI 智能合并
#   - PREV_TAG 从 package.json 的 fork 基线版本推导（fork 无 "merge upstream" commit 可 grep）
#
# 依赖: git, jq（gh 可选，用于解析上游 latest release；node/cargo 用于验证）

set -euo pipefail

# ── 失败时打印行号，便于 AI 立即定位中断点 ──────────────────────────────────
trap 'rc=$?; echo "[SCRIPT_FAILED] line $LINENO exit $rc" >&2' ERR

# ── 依赖检查 ─────────────────────────────────────────────────────────────────
for dep in git jq; do
  command -v "$dep" >/dev/null 2>&1 || { echo "错误: 找不到依赖 $dep" >&2; exit 1; }
done

# ── 锚定仓库根（用 cd+pwd 避免 dirname 相对路径分歧）────────────────────────
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(git -C "$SCRIPT_DIR" rev-parse --show-toplevel)"
cd "$REPO_ROOT"

# ── 参数解析 ─────────────────────────────────────────────────────────────────
CHECK_ONLY=false
NO_VERIFY=false
TARGET_TAG=""
for arg in "$@"; do
  case "$arg" in
    --check-only) CHECK_ONLY=true ;;
    --no-verify)  NO_VERIFY=true ;;
    v*)           TARGET_TAG="$arg" ;;
    *)            echo "未知参数: $arg" >&2; exit 1 ;;
  esac
done

# ── 0. 确认目标 tag + 推导 PREV_TAG ──────────────────────────────────────────
echo "[0/5] 拉取上游 tags..."
git fetch upstream --tags --force --quiet

# 上游最新 release tag：优先 gh（与 release skill 口径一致，取的是 release 而非任意 tag），
# 兜底用本地 tag（fetch 后已含上游 tag）。严格 3 段 semver，自动排除 fork 的 -fa.N 与上游 -rc/-alpha 预发布。
if [[ -z "$TARGET_TAG" ]]; then
  TARGET_TAG="$(gh api repos/clash-verge-rev/clash-verge-rev/releases/latest --jq '.tag_name' 2>/dev/null || true)"
  if ! [[ "$TARGET_TAG" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    TARGET_TAG="$(git tag --sort=-v:refname | grep -E '^v[0-9]+\.[0-9]+\.[0-9]+$' | head -1)"
  fi
fi
[[ -z "$TARGET_TAG" ]] && { echo "错误: 找不到上游 v*.*.* release tag" >&2; exit 1; }
git rev-parse --verify --quiet "refs/tags/${TARGET_TAG}^{commit}" >/dev/null \
  || { echo "错误: tag $TARGET_TAG 不存在（先 git fetch upstream --tags）" >&2; exit 1; }

# fork 当前基线：package.json 的 version 去掉 -fa.N 后缀即上游基线版本。
# 例: 2.4.7-fa.1043 → v2.4.7。fork 没有 "merge upstream" commit 可 grep，版本字段是唯一可靠来源。
FORK_VERSION="$(jq -r '.version' package.json)"
# 校验版本格式，避免误改后推导出 vunknown 之类的无效基线（git diff 会被 || true 静默吞掉）
if ! [[ "$FORK_VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-fa\.[0-9]+)?$ ]]; then
  echo "错误: package.json version 格式异常: $FORK_VERSION" >&2
  echo "      预期 X.Y.Z 或 X.Y.Z-fa.N（如 2.4.7 / 2.4.7-fa.1043）；先修正 version 字段再重跑" >&2
  exit 1
fi
PREV_TAG="v${FORK_VERSION%%-fa.*}"

BRANCH="fa/${TARGET_TAG}-fa.0"

echo "    fork 版本 : $FORK_VERSION"
echo "    上次基线  : $PREV_TAG"
echo "    目标 tag  : $TARGET_TAG"
echo "    目标分支  : $BRANCH"

if [[ "$PREV_TAG" == "$TARGET_TAG" ]]; then
  echo ""
  echo "已是最新：fork 基线 $PREV_TAG 已等于上游最新 release $TARGET_TAG，无需同步。"
  exit 1
fi

# ── 1. 预检：上游 ∩ fork 改动文件 ───────────────────────────────────────────
echo ""
echo "[1/5] 预检交叉修改文件..."

# 上游本次区间改动的文件
UPSTREAM_CHANGED="$(git diff --name-only "$PREV_TAG" "$TARGET_TAG" 2>/dev/null || true)"
# fork 相对上次基线改动的文件（含新增；新增文件不会冲突，但纳入便于 AI 排查连带影响）
FORK_MODIFIED="$(git diff --name-only "$PREV_TAG" HEAD 2>/dev/null || true)"

# 双方都改过 → git 自动合并即便成功也需语义复审（skill Step 2-D）
OVERLAP="$(comm -12 \
  <(echo "$UPSTREAM_CHANGED" | grep -v '^$' | sort) \
  <(echo "$FORK_MODIFIED"    | grep -v '^$' | sort) || true)"

if [[ -n "$OVERLAP" ]]; then
  echo "    ⚠️  双方均有修改的文件（合并时需智能分析 / 编译期审查）:"
  echo "$OVERLAP" | sed 's/^/      /'
else
  echo "    无交叉修改文件"
fi

if $CHECK_ONLY; then
  echo ""; echo "预检完成（--check-only）"; exit 0
fi

# ── 2. 创建分支（先排查未完成的 merge）──────────────────────────────────────
echo ""
echo "[2/5] 分支 $BRANCH..."

# 上次脚本中途失败可能留下 MERGE_HEAD；继续 merge 会让 auto_resolve 在已解决文件上二次应用，
# 造成静默回退。硬退出让 AI / 用户判断如何收尾。
if [[ -e "$REPO_ROOT/.git/MERGE_HEAD" ]]; then
  echo "错误: 检测到未完成的 merge（.git/MERGE_HEAD 存在）" >&2
  echo "      可能是上次脚本中途失败遗留。请先 git merge --abort 或人工解决冲突后再重跑" >&2
  echo "      （脚本不自动 abort，避免误删已完成的 AI 智能合并）。" >&2
  exit 1
fi

if git show-ref --verify --quiet "refs/heads/$BRANCH"; then
  echo "    分支已存在，继续在其上工作"
  git checkout "$BRANCH"
else
  git checkout -b "$BRANCH"
fi

# ── 3. 合并 ──────────────────────────────────────────────────────────────────
echo ""
echo "[3/5] 合并 $TARGET_TAG..."
MERGE_OK=true
git merge "$TARGET_TAG" --no-edit 2>&1 || MERGE_OK=false

CONFLICT_FILES=""
if ! $MERGE_OK; then
  CONFLICT_FILES="$(git diff --name-only --diff-filter=U)"
  CONFLICT_COUNT="$(echo "$CONFLICT_FILES" | grep -cv '^$' || true)"
  echo "    冲突文件: ${CONFLICT_COUNT} 个"
fi

# ── 4. 自动解决纯 fork 所有的冲突 ───────────────────────────────────────────
# 注意: clash-verge-rev fork 走"// FORK 标记 + 智能合并"哲学，绝大多数冲突文件
#       (CI/版本/功能集成) 都需要"保留 fork + 吸收上游"的语义合并，不能无脑 --ours。
#       脚本只对 Changelog.md 自动 --ours —— 它是纯 fork 维护的发布说明，且会被
#       release skill 的 prepare-release.sh 重新生成，上游内容无关紧要。
echo ""
echo "[4/5] 自动解决纯 fork 所有的冲突..."

SCRIPT_AUTO_RESOLVED=()

auto_resolve_ours() {
  local f="$1"; local reason="$2"
  if echo "$CONFLICT_FILES" | grep -qxF "$f"; then
    echo "    $f → --ours（${reason}）"
    git checkout --ours "$f" && git add "$f"
    SCRIPT_AUTO_RESOLVED+=("$f|ours|${reason}")
  fi
}

auto_resolve_ours "Changelog.md" "fork 发布说明，由 prepare-release.sh 重新生成"

# ── 对剩余冲突分类，供 skill 决策矩阵消费 ────────────────────────────────────
REMAINING="$(git diff --name-only --diff-filter=U 2>/dev/null || true)"

# 含 // FORK 标记的冲突文件 → 保留 fork 块是首要原则
FORK_MARKER_FILES=()
# src/locales/*/*.json → i18n 源，并集合并（fork 新增 key + 上游改动都保留）
I18N_SOURCE_FILES=()
# src/types/generated/* → i18n 生成产物，不可手改；先合 locales 再 pnpm i18n:types 重新生成
REGEN_I18N=()
# 版本 / 签名配置 → 保留 fork 的 version/pubkey/endpoints，接受上游依赖与新配置
VERSION_FILES=()
if [[ -n "$REMAINING" ]]; then
  while IFS= read -r f; do
    [[ -z "$f" ]] && continue
    case "$f" in
      src/locales/*/*.json)  I18N_SOURCE_FILES+=("$f"); continue ;;
      src/types/generated/*) REGEN_I18N+=("$f"); continue ;;
      package.json|src-tauri/Cargo.toml|src-tauri/tauri.conf.json) VERSION_FILES+=("$f") ;;
    esac
    # git stage 2 (ours/HEAD) 内含 // FORK 标记 → 标给 AI 重点保留
    # 注意: 只能检出"冲突块内带标记"的改动；fork 的 URL 替换 / 末尾新增函数(无标记)检不出，
    #       靠 skill Step 3-B① 用 git diff PREV_TAG..HEAD 人工补扫。
    if git show ":2:$f" 2>/dev/null | grep -q "FORK:"; then
      FORK_MARKER_FILES+=("$f")
    fi
  done <<< "$REMAINING"
fi

# ── 机器可读 hint 给 AI（skill Step 2 / 3 / 5-C 消费）───────────────────────
emit_ai_hints() {
  echo ""
  echo "--- AI_HINTS ---"
  echo "PREV_TAG=${PREV_TAG}"
  echo "TARGET_TAG=$TARGET_TAG"
  echo "BRANCH=$BRANCH"

  if [[ ${#SCRIPT_AUTO_RESOLVED[@]} -gt 0 ]]; then
    echo "SCRIPT_AUTO_RESOLVED:"
    printf '  %s\n' "${SCRIPT_AUTO_RESOLVED[@]}"
  else
    echo "SCRIPT_AUTO_RESOLVED: (none)"
  fi

  echo "GIT_OVERLAP_FILES:"   # 自动合并成功但双方都改过 → 必做连带影响复审
  if [[ -n "$OVERLAP" ]]; then echo "$OVERLAP" | sed 's/^/  /'; else echo "  (none)"; fi

  echo "FORK_MARKER_FILES:"   # 冲突文件含 // FORK 标记 → 保留 fork 块优先
  if [[ ${#FORK_MARKER_FILES[@]} -gt 0 ]]; then printf '  %s\n' "${FORK_MARKER_FILES[@]}"; else echo "  (none)"; fi

  echo "VERSION_FILES:"       # 保留 fork version/pubkey/endpoints，接受上游依赖
  if [[ ${#VERSION_FILES[@]} -gt 0 ]]; then printf '  %s\n' "${VERSION_FILES[@]}"; else echo "  (none)"; fi

  echo "I18N_SOURCE_FILES:"   # 并集合并（fork 新增 key + 上游改动都保留），合后跑 pnpm i18n:types
  if [[ ${#I18N_SOURCE_FILES[@]} -gt 0 ]]; then printf '  %s\n' "${I18N_SOURCE_FILES[@]}"; else echo "  (none)"; fi

  echo "REGEN_I18N:"          # 不可手改，pnpm i18n:types 重新生成
  if [[ ${#REGEN_I18N[@]} -gt 0 ]]; then printf '  %s\n' "${REGEN_I18N[@]}"; else echo "  (none)"; fi
  echo "--- END AI_HINTS ---"
}

# ── 输出剩余冲突（供 AI 智能合并）──────────────────────────────────────────
if [[ -n "$REMAINING" ]]; then
  echo ""
  echo "SMART_MERGE_REQUIRED"
  echo "CONFLICT_FILES:"
  echo "$REMAINING" | sed 's/^/  /'
  emit_ai_hints
  echo ""
  echo "以下文件需 AI 按 AI_HINTS 分类处理（FORK 标记→智能合并 / I18N_SOURCE→并集合并 / REGEN_I18N→重新生成 / VERSION→版本合并 / 其余→智能合并）:"
  echo "$REMAINING" | sed 's/^/  /'
  exit 2
fi

# 无残余冲突才提交
if ! $MERGE_OK; then
  git commit -m "chore: merge upstream $TARGET_TAG

Auto-resolved: Changelog.md (--ours)."
fi

emit_ai_hints

# ── 5. 验证（快速门禁；完整 lint/test/build 留给 skill Step 4）──────────────
echo ""
echo "[5/5] 验证..."
if $NO_VERIFY; then
  echo "    跳过验证（--no-verify）"
else
  VERIFY_OK=true

  if [[ -d node_modules ]]; then
    echo "    pnpm typecheck..."
    pnpm typecheck 2>&1 || VERIFY_OK=false
  else
    echo "    ⚠️  node_modules 缺失，跳过 typecheck（先 pnpm install）"
  fi

  echo "    cargo check --workspace..."
  cargo check --workspace 2>&1 || VERIFY_OK=false

  if ! $VERIFY_OK; then
    echo "验证失败 — 提示: 上游可能改了类型签名/依赖；检查 fork 独有文件是否需同步" >&2
    echo "  TS:   git diff $TARGET_TAG HEAD -- package.json && pnpm install" >&2
    echo "  Rust: git diff $TARGET_TAG HEAD -- src-tauri/Cargo.toml Cargo.toml" >&2
    exit 3
  fi
  echo "    验证通过 ✓"
fi

echo ""
echo "完成 ✓  分支: $BRANCH  HEAD: $(git rev-parse --short HEAD)"
echo "下一步: 完成 skill Step 4 完整验证 (pnpm i18n:types / lint / clippy-all / test) 后"
echo "        git push -u origin $BRANCH"
