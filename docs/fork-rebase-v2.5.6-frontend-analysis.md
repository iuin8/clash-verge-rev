# fork 前端 × 上游 v2.5.6：可替代 / 需重植 / 冲突面 结论

基线 fork `HEAD` = `c75c5b02`（`v2.5.2-fa.1010`），上游 `v2.5.6`。
三方合并结论用 `git merge-tree --write-tree HEAD v2.5.6` 实测，产物 tree = `34ca43e2`。

> 与 `docs/fork-rebase-v2.5.6-rust-analysis.md` 配套；本文是 `.claude/skills/upstream-sync/SKILL.md` 第 2/3 步的决策依据，不替代该 skill 要求的对话内 6 句汇报。
> ✅ = 已在本机复验；⚠️ = 子代理结论，尚未独立复验。

---

## 1. 合并产物不可编译（最高优先级）

即使 `git merge-tree` 报告"auto-merge 成功"，下列位置在合并产物里就已损坏：

| 位置（tree `34ca43e2`） | 症状 | 复验 |
|---|---|---|
| `src/pages/profiles.tsx:140` 与 `:146` | `const [batchMode, setBatchMode] = useState(false)` 重复声明 | ✅ TS2451 |
| `src/pages/profiles.tsx:1267` | `selected={(switchTarget ?? profiles.current) === item.uid}`，`switchTarget` 全文件未声明 | ✅ TS2304 |
| `src/pages/settings.tsx:13/24` | 只 import 了 `openExternalUrl`，第 24 行仍调 `openWebUrl` | ✅ TS2304 |
| `src/components/setting/mods/update-viewer.tsx:327` | 调用 `openUrl(...)`，但合并产物已无 `@tauri-apps/plugin-shell` 的 import | ✅ TS2304 + 包已被上游删除 |

结论：这三个前端冲突文件**必须手工决断**，不能"信 auto-merge"。

## 2. 结论表

| fork 前端改动 | 上游是否已覆盖 | 处置 | 依据（✅ 复验项） |
|---|---|---|---|
| 列表拖拽 `DndContext`/`SortableContext`/`SortableProfileItem` | 已覆盖（`@dnd-kit/react`） | 整体改用上游结构；`sortable-profile-item.tsx` 上游已删 | ✅ |
| `profileRectSortingStrategy` 自实现 | 无必要 | 删除（新架构内置） | ⚠️ |
| `onDragStart` + `localActiveOrder` 防回弹 | 未覆盖，语义仍需要 | 保留概念，但 index 口径必须改成"渲染顺序数组"（上游用 `profileItems[oldIndex/newIndex]`） | ⚠️ |
| `switchTarget` | 上游 v2.5.6 仍保留 | 二选一必须显式决断（fork 侧在 d9db6dfe 丢弃了它） | ✅ |
| `batchSelected` 重命名 | 未覆盖（上游叫 `selectedProfiles`） | 保留 fork 命名，手工解冲突 | ✅ |
| `MergeOrderBar` 组件 | 未覆盖 | **已是死代码**（`src/` 内零引用，仅自引用）→ 直接删除；连带 `@dnd-kit/core`/`sortable`/`utilities` 依赖问题一并消失 | ✅ |
| `ConflictViewer` + 工具栏冲突徽标 | 未覆盖 | 保留（新增文件，零冲突；`profiles.tsx:46/1278` 有引用） | ✅ |
| `update-viewer` 的 `resolvedVersion` + fork release URL | 部分覆盖 | 保留 `export resolveRemoteVersion` 补丁；按钮改走上游 `openUrlWithNotice` + fork URL | ✅ |
| `settings.tsx` 的 `openWebUrl(iuin8)` | 语义冲突 | 必须改 `openExternalUrl(...).catch(onError)` | ✅ |
| `cmds.ts` 三个 merge 命令 / `global.d.ts` `ConflictEntry` | 未覆盖 | 保留（自动合并干净） | ⚠️ |
| i18n 新增 key | 未覆盖 | 保留；`profileSwitchFailed` 与上游 `errors.switchFailed` 重复，建议换用上游 key | ⚠️ |
| `update.ts` `export resolveRemoteVersion` | 上游同函数未导出 | 保留最小补丁 | ⚠️ |
| `unlock.tsx` | 上游重写 218 行、零冲突 | 整份取上游（见 §4） | ✅ |

## 3. 用上游新能力替换 fork 实现

| 用 X | 替换 Y |
|---|---|
| `subscribeVergeEvents({...})` | `listen()` + `Promise.allSettled` 清理 |
| `fetchProfilesIntoCache()` | `fetchCacheData(['getProfiles'], getProfiles)` |
| `revalidateQuery(['getRuntimeLogs'])` + `mutateProfiles()` | `revalidateQueries([[...], [...]])` |
| `errorDetail(err)` | 上游 notice-service 的等价导出 |
| `isValidUrl(url)` | `/^https?:\/\//i` |
| `getProxyView`（单次 IPC，含 providers） | `calcuProxies` + `calcuProxyProviders` |
| `getRuntimeState` | `getRunningMode` |
| `openExternalUrl` | `openWebUrl`（上游已删） |
| `check_media_unlock_item` | unlock 整表重测 |
| vitest / `scripts/dev*.mjs` | 可给 multi-merge 逻辑补单测；dev 脚本改用 `node scripts/dev.mjs` |
| perf harness `scripts/perf/*` | **不适用**（上游 GUIDE 限定唯一场景为 macOS 首页流量图） |

## 4. unlock 语义变化（必须整份吸收）

- `check_media_unlock` 由无参变 `check_media_unlock(on_complete: Channel<UnlockItem>)`（✅ 已复验 `v2.5.6:src-tauri/src/cmd/media_unlock_checker/mod.rs`）。
- `get_unlock_items` 返回类型变化；新增 `check_media_unlock_item(name)`。
- 上游把协议实现迁进新 crate `crates/clash-verge-media-unlock`，仓库侧只留 thin `mod.rs` 转发（✅ 已复验目录只剩 `mod.rs`）。
- 结论：前端 `unlock.tsx` + `cmd/media_unlock_checker/mod.rs` + 新 crate 必须**同侧吸收**；只取一侧会 IPC 反序列化失败。fork 前端本身零额外改动。

## 5. i18n 流程

1. git 自动合并 locales（实测零冲突）→
2. `pnpm i18n:format`（en 为基线对齐 13 个 locale）→
3. `pnpm i18n:types`（重生成 `src/types/generated/*`，禁止手改）→
4. `pnpm i18n:check`

⚠️ 陷阱：`i18n:format` **清不掉死 key**（`findKeyInSources` 会在 `src/**` 里匹配到 `generated/i18n-keys.ts` 中逐字列出的全部 key，故 unused 恒为 0，实测 en total=806 / unused=0）。`profiles.merge.activate|activated` 等死 key 只能手动删。

## NEEDS_USER_REVIEW

1. `MergeOrderBar` 是否复活（当前是死代码）。
2. `switchTarget` 语义取舍：fork"所有激活卡=selected" vs 上游"仅 current=selected"。
3. 是否保留双区（激活/非激活）—— 上游是单列表，保留则必须自定义 index 映射。
4. 合并顺序权威源：前端 `localActiveOrder` vs 后端 `profiles.merged`；`set_merged_profiles` 是否按传入顺序持久化**未验证**。
5. `@dnd-kit/react` 的 `group` 跨组隔离**未验证**。
6. `resolvedVersion` 是否仍需，取决于 fork `latest.json` 的 `version`/`tag_name` 形态 —— **未验证**。
