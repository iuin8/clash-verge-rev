<!-- Release-notes skeleton. Entries accumulate in ./Changelog.md as PRs land;
     before publishing a release, distill them:
     - user-visible changes only — no dev tooling, CI fixes, or refactors whose
       effect is already covered by another entry
     - describe outcomes, not internals: no internal module names, IPC or
       protocol details, or implementation mechanics — write what the user
       observes, not how the code achieves it
     - one line per theme: merge entries that share a root cause
     - each entry fits a single readable line (about 60 CJK characters); if it
       overflows, it is packing multiple themes or detail that belongs in the
       linked issue/PR — split it or drop the detail
     - line grammar: open with a verb (修复/新增/优化), with the platform
       first (Windows/macOS/Linux) when the change is platform-specific;
       merge sub-symptoms as 主题：症状、症状; full-width punctuation,
       「」 for UI labels, backticks for config keys and commands
     - platform entries sit under their platform heading (bold, emoji
       prefixed, ordered Windows/macOS/Linux, combined for pairs like
       macOS/Linux) at the end of their section;
       scripts-workflow/group_platforms.sh regroups, --check verifies
     HTML comments are stripped by the update dialog's sanitizer and hidden on
     GitHub, so they never reach users. -->
## v2.5.10-fa.5

> [!IMPORTANT]
> 这是基于上游 v2.5.10 的个人 fork 版本。

### 🚀 优化改进

- 自动订阅更新合并为一次组合生成/内核应用，减少重复重载
- 组合刷新状态、批量模式边界和刷新时间显示更稳定
- 将组合逻辑隔离到独立模块，降低后续同步上游时的冲突面

## v2.5.10-fa.4

> [!IMPORTANT]
> 这是基于上游 v2.5.10 的个人 fork 版本。

### ✨ 新增功能

- 组合卡片和详情显示最近刷新时间（相对时间，悬停可看完整时间）

### 🚀 优化改进

- 单个成员订阅的自动更新会合并为一次组合重建和一次内核应用，避免多成员同时到期时重复重载
- 自动更新只刷新成员文件，组合由 single-flight 防抖 materializer 统一生成

## v2.5.10-fa.3

> [!IMPORTANT]
> 这是基于上游 v2.5.10 的个人 fork 版本。

### ✨ 新增功能

- 组合订阅支持完整手动刷新：先更新组合内全部远端订阅，再重新生成并应用组合
- 组合详情和卡片菜单始终提供刷新入口，不再只在 stale 时显示

### 🚀 优化改进

- 远端成员逐个更新；单个成员失败不阻塞其余成员，并提示失败数量
- 完整刷新只做一次最终生成和应用，避免多成员重复重载内核

## v2.5.10-fa.2

> [!IMPORTANT]
> 这是基于上游 v2.5.10 的个人 fork 版本。

### ✨ 新增功能

- 新增「组合订阅」：多选订阅后生成一个可单选的普通本地配置，成员顺序即合并顺序，成员更新会自动刷新
- 旧的合并选择会在启动时迁移为「组合订阅」，不再保留隐藏的全局合并状态

### 🚀 优化改进

- 组合订阅使用单文件覆盖式更新，不增加订阅列表条目；删除组合不会删除原订阅

## v2.5.10-fa.1

> [!IMPORTANT]
> 这是基于上游 v2.5.10 的个人 fork 版本。

### 🚀 优化改进

- 在 v2.5.10 新架构上重做多订阅合并：订阅管理页可选择、排序并应用多份配置，冲突集中展示
- 随应用打包的 alpha 内核继续使用本 fork 的发布资产，升级逻辑不再因 rolling tag 被静默跳过
- Linux 暂只发布 DEB：当前 Tauri RPM bundler 不接受 `-fa.N` 版本号，避免发布不可安装的 RPM

## v2.5.10

<details>
<summary><strong> 🐞 修复问题 </strong></summary>

- 修复全局模式缺少 `GLOBAL` 策略组，无法单独选择全局出口
- 修复服务模式下，SSH 节点的 `private-key` 使用私钥文件时内核无法启动
- 修复服务模式下，内核更新的 GEO 数据库不会同步回应用目录，导致使用新分类的规则无法通过校验
- 修复订阅的 DNS 配置变化后，「DNS 覆写」实际已关闭但开关仍显示开启且没有提示

**🖥️ Windows**

- Windows：修复安装程序重新安装或覆盖时未提示退出运行中应用，导致卸载报错或覆盖失败

</details>

<details>
<summary><strong> ✨ 新增功能 </strong></summary>


</details>

<details>
<summary><strong> 🚀 优化改进 </strong></summary>


</details>
