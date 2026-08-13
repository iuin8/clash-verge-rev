## v2.5.2-fa.1009

> [!IMPORTANT]
> 这是基于上游 v2.5.2-fa.1008 的个人 fork 版本。

### ✨ 新增功能

- manage system SSH config via ssh-config YAML key

---


## v2.5.2-fa.1008

> [!IMPORTANT]
> 这是基于上游 v2.5.2-fa.1007 的个人 fork 版本。

### 🐞 修复问题

- refine merge conflict indicators

---


## v2.5.2-fa.1007

> [!IMPORTANT]
> 这是基于上游 v2.5.2-fa.1006 的个人 fork 版本。

### 🐞 修复问题

- surface primary merge conflicts

---


## v2.5.2-fa.1006

> [!IMPORTANT]
> 这是基于上游 v2.5.2-fa.1005 的个人 fork 版本。

### 🐞 修复问题

- restore merge conflict trigger

### ✨ 新增功能

- highlight primary merge profile

---


## v2.5.2-fa.1005

> [!IMPORTANT]
> 这是基于上游 v2.5.2-fa.1004 的个人 fork 版本。

### 🐞 修复问题

- preserve selected card width

---


## v2.5.2-fa.1004

> [!IMPORTANT]
> 这是基于上游 v2.5.2-fa.1003 的个人 fork 版本。

### 🐞 修复问题

- resolve service version before prebuild on Windows

---


## v2.5.2-fa.1003

> [!IMPORTANT]
> 这是基于上游 v2.5.2-fa.1002 的个人 fork 版本。

### 🐞 修复问题

- align service binary version with IPC client
- Revert "fix: pin service IPC binary version"
- pin service IPC binary version

---


## v2.5.2-fa.1002

> [!IMPORTANT]
> 这是基于上游 v2.5.2-fa.1001 的个人 fork 版本。

### 🐞 修复问题

- ad-hoc sign macOS release bundles

---


## v2.5.2-fa.1001

> [!IMPORTANT]
> 这是基于上游 v2.5.2 的个人 fork 版本。

### 🐞 修复问题

- grant release job id-token/attestations for upstream attest steps
- 修复 fork 客户端版本号比较 (A 方案)
- 只输出 version 字段避免 duplicate field 错误
- 恢复 update.json 的 version 字段,修复更新对话框
- 拖动排序后重跑 enhance pipeline
- 拖动排序在切换 tab 后回退
- 修复拖动排序失效
- 订阅切换失败时正确回滚 UI 状态并 restore mihomo 配置
- docs(merge): record shipped rule-provider fix state
- harden multi-profile merge edge cases
- harden multi-profile merge behavior
- update build monitoring interval to every 3 minutes
- Revert "fix: use MetaCubeX upstream for alpha mihomo sidecar prebuild"
- use MetaCubeX upstream for alpha mihomo sidecar prebuild
- force release body sync after publish
- simplify update_tag job dependencies and adjust release name format
- update_tag must wait for all release jobs to complete
- add missing TAG_NAME env variable in update_tag job
- add missing release body update step in update_tag job
- update draft release body to include 'Draft' prefix for clarity
- replace draft release update with action-gh-release for streamlined uploads
- add make_latest parameter to force update release body
- correct UPDATE_LOGS environment variable usage in release workflow
- update release body generation to use environment variable for update logs
- 简化 release 下载链接，只显示实际构建的平台（Windows x64, macOS ARM64, Linux x64）
- 修复 release 下载链接指向错误仓库的问题，使用 github.repository 变量
- 移除 release 名称中的 'Clash Verge Rev' 前缀，只保留版本号
- update GitHub repository link to point to the correct owner
- 修复版本号显示和更新链接问题
- 移除冗余version字段，用name存semver版本号避免duplicate field错误
- 使用releases API替代tags API，避免选中上游继承的tag
- 更新版本标签格式以符合semver规范
- 拖拽时过滤已失活配置，防止非激活项出现在激活区
- 修复拖拽活动区消失和幻影现象
- 修复拖拽和切换时的状态同步问题
- surface conflict-load errors, stabilise primary-card identity
- enforce drag constraint, fix Done visibility, apply active highlight to selected cards
- spec compliance fixes for multi-select merge mode
- ci: re-enable release-update job and fix fork updater config
- ci: disable unused jobs (linux-arm, fixed-webview2, winget, telegram, upstream-updater)
- 修复多配置文件合并时的格式问题
- resolve clippy errors in multi_merge and enhance pipeline
- docs: 修复package.md中的空格格式问题
- 修复版本标签匹配逻辑以支持带后缀的标签

### ✨ 新增功能

- add upstream-sync skill and script for merging upstream changes
- add upstream auto-sync workflow with AI assistance
- add GitNexus skills and update AGENTS.md for enhanced code intelligence
- 添加非交互模式支持 --yes 参数以支持 CI/CD 和 agent 自动化
- 添加 workflow_call 支持以允许其他 workflow 调用 updater
- add automated release process with Changelog update and tag creation
- ci: 更新发布版本号至v2.4.7.1017并添加版本号自动更新逻辑
- 非激活区禁止拖拽，仅支持点击切换
- 重构拖拽排序为本地状态驱动，修复回弹问题
- 改进拖拽交互的视觉反馈和逻辑
- 添加多合并模式下主卡片高亮显示功能
- docs: 更新package.md添加macOS签名问题说明
- wire MergeOrderBar and ConflictViewer for multi-profile merge activation
- add MergeOrderBar component for drag-to-reorder merge priority
- add ConflictViewer dialog for merge conflict display
- add TS types and wrappers for multi-profile merge commands
- add set_merged_profiles, clear_merged_profiles, get_merge_conflicts IPC commands
- wire multi-profile merge path in enhance pipeline
- add multi_profile_merge function with conflict logging
- add merged field to IProfiles for multi-profile activation
- add merge activation i18n keys
- chore: 更新.gitignore文件，添加.omc目录忽略
- docs: 添加打包文档说明

### 🚀 优化改进

- shrink fork merge overlay
- 移除右键"使用"菜单项及连带 dead code
- 重构多选和合并功能，移除批量模式
- docs(specs): 更新多配置合并设计文档，优化UI交互和冲突处理
- 简化平台架构映射并更新文件名格式 refactor(enhance): 优化代码格式和合并逻辑的可读性

---


## v2.5.1-fa.1001

> [!IMPORTANT]
> 这是基于上游 v2.5.1-fa.0 的个人 fork 版本。

### 🐞 修复问题

- 修复 fork 客户端版本号比较 (A 方案)
- 只输出 version 字段避免 duplicate field 错误
- 恢复 update.json 的 version 字段,修复更新对话框
- 拖动排序后重跑 enhance pipeline
- 拖动排序在切换 tab 后回退
- 修复拖动排序失效
- 订阅切换失败时正确回滚 UI 状态并 restore mihomo 配置
- docs(merge): record shipped rule-provider fix state
- harden multi-profile merge edge cases
- harden multi-profile merge behavior
- update build monitoring interval to every 3 minutes
- Revert "fix: use MetaCubeX upstream for alpha mihomo sidecar prebuild"
- use MetaCubeX upstream for alpha mihomo sidecar prebuild
- force release body sync after publish
- simplify update_tag job dependencies and adjust release name format
- update_tag must wait for all release jobs to complete
- add missing TAG_NAME env variable in update_tag job
- add missing release body update step in update_tag job
- update draft release body to include 'Draft' prefix for clarity
- replace draft release update with action-gh-release for streamlined uploads
- add make_latest parameter to force update release body
- correct UPDATE_LOGS environment variable usage in release workflow
- update release body generation to use environment variable for update logs
- 简化 release 下载链接，只显示实际构建的平台（Windows x64, macOS ARM64, Linux x64）
- 修复 release 下载链接指向错误仓库的问题，使用 github.repository 变量
- 移除 release 名称中的 'Clash Verge Rev' 前缀，只保留版本号
- update GitHub repository link to point to the correct owner
- 修复版本号显示和更新链接问题
- 移除冗余version字段，用name存semver版本号避免duplicate field错误
- 使用releases API替代tags API，避免选中上游继承的tag
- 更新版本标签格式以符合semver规范
- 拖拽时过滤已失活配置，防止非激活项出现在激活区
- 修复拖拽活动区消失和幻影现象
- 修复拖拽和切换时的状态同步问题
- surface conflict-load errors, stabilise primary-card identity
- enforce drag constraint, fix Done visibility, apply active highlight to selected cards
- spec compliance fixes for multi-select merge mode
- ci: re-enable release-update job and fix fork updater config
- ci: disable unused jobs (linux-arm, fixed-webview2, winget, telegram, upstream-updater)
- 修复多配置文件合并时的格式问题
- resolve clippy errors in multi_merge and enhance pipeline
- docs: 修复package.md中的空格格式问题
- 修复版本标签匹配逻辑以支持带后缀的标签

### ✨ 新增功能

- add upstream-sync skill and script for merging upstream changes
- add upstream auto-sync workflow with AI assistance
- add GitNexus skills and update AGENTS.md for enhanced code intelligence
- 添加非交互模式支持 --yes 参数以支持 CI/CD 和 agent 自动化
- 添加 workflow_call 支持以允许其他 workflow 调用 updater
- add automated release process with Changelog update and tag creation
- ci: 更新发布版本号至v2.4.7.1017并添加版本号自动更新逻辑
- 非激活区禁止拖拽，仅支持点击切换
- 重构拖拽排序为本地状态驱动，修复回弹问题
- 改进拖拽交互的视觉反馈和逻辑
- 添加多合并模式下主卡片高亮显示功能
- docs: 更新package.md添加macOS签名问题说明
- wire MergeOrderBar and ConflictViewer for multi-profile merge activation
- add MergeOrderBar component for drag-to-reorder merge priority
- add ConflictViewer dialog for merge conflict display
- add TS types and wrappers for multi-profile merge commands
- add set_merged_profiles, clear_merged_profiles, get_merge_conflicts IPC commands
- wire multi-profile merge path in enhance pipeline
- add multi_profile_merge function with conflict logging
- add merged field to IProfiles for multi-profile activation
- add merge activation i18n keys
- chore: 更新.gitignore文件，添加.omc目录忽略
- docs: 添加打包文档说明

### 🚀 优化改进

- 移除右键"使用"菜单项及连带 dead code
- 重构多选和合并功能，移除批量模式
- docs(specs): 更新多配置合并设计文档，优化UI交互和冲突处理
- 简化平台架构映射并更新文件名格式 refactor(enhance): 优化代码格式和合并逻辑的可读性

---


## v2.4.7-fa.1043

> [!IMPORTANT]
> 这是基于上游 v2.4.7 的个人 fork 版本。

### 🐞 修复问题

- 修复 fork 客户端版本号比较 (A 方案)

---

## v2.4.7-fa.1042

> [!IMPORTANT]
> 这是基于上游 v2.4.7 的个人 fork 版本。

### 🐞 修复问题

- 只输出 version 字段避免 duplicate field 错误

---

## v2.4.7-fa.1041

> [!IMPORTANT]
> 这是基于上游 v2.4.7 的个人 fork 版本。

### 🐞 修复问题

- 恢复 update.json 的 version 字段,修复更新对话框

---

## v2.4.7-fa.1040

> [!IMPORTANT]
> 这是基于上游 v2.4.7 的个人 fork 版本。

### 🐞 修复问题

- 拖动排序后重跑 enhance pipeline

---

## v2.4.7-fa.1039

> [!IMPORTANT]
> 这是基于上游 v2.4.7 的个人 fork 版本。

### 🐞 修复问题

- 拖动排序在切换 tab 后回退

---

## v2.4.7-fa.1038

> [!IMPORTANT]
> 这是基于上游 v2.4.7 的个人 fork 版本。

### 🐞 修复问题

- 修复拖动排序失效

### 🚀 优化改进

- 移除右键"使用"菜单项及连带 dead code

---

## v2.4.7-fa.1037

> [!IMPORTANT]
> 这是基于上游 v2.4.7 的个人 fork 版本。

### 🐞 修复问题

- 订阅切换失败时正确回滚 UI 状态并 restore mihomo 配置

---

## v2.4.7-fa.1036

> [!IMPORTANT]
> 这是基于上游 v2.4.7 的个人 fork 版本。

### 🐞 修复问题

- docs(merge): record shipped rule-provider fix state
- harden multi-profile merge edge cases
- harden multi-profile merge behavior
- update build monitoring interval to every 3 minutes

---

## v2.4.7-fa.1035

> [!IMPORTANT]
> 这是基于上游 v2.4.7 的个人 fork 版本。

### 🐞 修复问题

- Revert "fix: use MetaCubeX upstream for alpha mihomo sidecar prebuild"

---

## v2.4.7-fa.1034

> [!IMPORTANT]
> 这是基于上游 v2.4.7 的个人 fork 版本。

### 🐞 修复问题

- use MetaCubeX upstream for alpha mihomo sidecar prebuild

---

## v2.4.7-fa.1033

> [!IMPORTANT]
> 这是基于上游 v2.4.7 的个人 fork 版本。

### 📝 更新内容

- docs: enhance release process with upstream version check and user notification
- ci: deterministic updater channel refresh on release

---

## v2.4.7-fa.1032

> [!IMPORTANT]
> 这是基于上游 v2.4.7 的个人 fork 版本。

### 🐞 修复问题

- force release body sync after publish

---

## v2.4.7-fa.1031

> [!IMPORTANT]
> 这是基于上游 v2.4.7 的个人 fork 版本。

### 📝 更新内容

- ci: enforce Changelog.md entry validation before release build

---

## v2.4.7-fa.1030

> [!IMPORTANT]
> 这是基于上游 v2.4.7 的个人 fork 版本。

### 🐞 修复问题

- simplify update_tag job dependencies and adjust release name format

---

## v2.4.7-fa.1026

> [!IMPORTANT]
> 这是基于上游 v2.4.7 的个人 fork 版本。

### 🐞 修复问题

- 修复 release body 未更新的问题，通过给 draft release 添加 'Draft' 前缀来区分草稿和最终版本

---

## v2.4.7-fa.1025

> [!IMPORTANT]
> 这是基于上游 v2.4.7 的个人 fork 版本。

### 🐞 修复问题

- fix release body not updating by adding make_latest parameter to softprops/action-gh-release

---

## v2.4.7-fa.1024

> [!IMPORTANT]
> 这是基于上游 v2.4.7 的个人 fork 版本。

### 🐞 修复问题

- correct UPDATE_LOGS environment variable usage in release workflow

---

## v2.4.7-fa.1023

> [!IMPORTANT]
> 这是基于上游 v2.4.7 的个人 fork 版本。

### 🐞 修复问题

- update release body generation to use environment variable for update logs

### ✨ 新增功能

- add upstream auto-sync workflow with AI assistance

---

## v2.4.7-fa.1022

> [!IMPORTANT]
> 这是基于上游 v2.4.7 的个人 fork 版本。

### 🐞 修复问题

- 简化 release 下载链接，只显示实际构建的平台（Windows x64, macOS ARM64, Linux x64）
- 修复 release 下载链接指向错误仓库的问题，使用 github.repository 变量
- 移除 release 名称中的 'Clash Verge Rev' 前缀，只保留版本号

### ✨ 新增功能

- add GitNexus skills and update AGENTS.md for enhanced code intelligence
- 添加非交互模式支持 --yes 参数以支持 CI/CD 和 agent 自动化

---

## v2.4.7-fa.1021

> [!IMPORTANT]
> 这是基于上游 v2.4.7 的个人 fork 版本。

### 🐞 修复问题

- update GitHub repository link to point to the correct owner

### ✨ 新增功能

- 添加 workflow_call 支持以允许其他 workflow 调用 updater
- add automated release process with Changelog update and tag creation

---

## v2.4.7.1001

> [!IMPORTANT]
> 这是基于上游 v2.4.7 的个人 fork 版本，主要修复了更新系统和配置管理相关问题。

### 🐞 修复问题

- 修复版本号显示和更新链接问题
- 修复 updater 选中上游继承的 tag 问题，使用 releases API 替代 tags API
- 修复 updater 中 duplicate field 'version' 错误，移除冗余字段
- 修复更新版本标签格式以符合 semver 规范
- 修复拖拽时过滤已失活配置，防止非激活项出现在激活区
- 修复拖拽活动区消失和幻影现象
- 修复拖拽和切换时的状态同步问题
- 修复 GitHub 仓库链接指向正确的 owner

### ✨ 新增功能

- 非激活区禁止拖拽，仅支持点击切换
- 改进拖拽交互的视觉反馈和逻辑

### 🚀 优化改进

- 重构拖拽排序为本地状态驱动，修复回弹问题
- 重构多选和合并功能，移除批量模式
- 更新发布工作流以支持带连字符的标签格式
- 添加版本号自动更新逻辑

---

## v2.4.7
## v2.5.1

> [!IMPORTANT]
> 继2.4.6以来继续优化修复问题，是 bug 问题最少的版本；建议所有用户立即升级。
> 关于版本的说明：Clash Verge 版本号遵循 x.y.z：x 为重大架构变更，y 为功能新增，z 为 Bug 修复。

### 🐞 修复问题

- 修复 Windows 管理员身份运行时开关 TUN 模式异常
- 修复静默启动与自动轻量模式存在冲突
- 修复进入轻量模式后无法返回主界面
- 切换配置文件偶尔失败的问题
- 修复节点或模式切换出现极大延迟的回归问题
- 修复代理关闭的情况下，网站测试依然会走代理的问题
- 修复 Gemini 解锁测试不准确的情况
- 修复删除订阅后状态栏未更新的问题

### ✨ 新增功能

- 升级 Mihomo 内核

### 🚀 优化改进

- 优化订阅错误通知，仅在手动触发时
- 隐藏日志中的订阅信息
- 优化部分界面文案文本
- 优化切换节点时的延迟
- 优化托盘退出快捷键显示
- 优化首次启动节点信息刷新
- Linux 默认使用内置窗口控件
- 实现排除自定义网段的校验
- 移除冗余的自动备份触发条件
- 恢复内置编辑器对 mihomo 配置的语法提示
- 网站测试使用真实 TLS 握手延迟
- 系统代理指示器(图标)使用真实代理状态
- 系统代理开关指示器增加校验是否指向 Verge
- 系统代理开关修改为乐观更新模式，提升用户体验
- 备份设置功能异常
- 修复 Windows 节点交互异常 
## v2.5.2

### 🐞 修复问题

- 修复 macOS 托盘速率显示样式异常
- 修复订阅 TLS 1.0/1.1 等过旧协议时显示更明确错误原因
- 修复 gzip 压缩订阅响应被当作无效 YAML 导致导入失败的问题
- 修复订阅 URL 使用空密码 Basic Auth 时未发送认证信息的问题
- 修复 Linux 托盘可能与其他 Tauri 程序冲突导致的图标异常
- 修复前端连接页面导致的内存泄漏
- 修复 macOS 12（Monterey）首页 IP 卡片兼容性问题
- 修复首页代理卡片可能错误显示通信异常的问题
- 修复 Fake-IP 模式开启 IPv6 后未生成 fake-ip-range6 的问题
- 修复 DNS 覆写的高级模式无法正常编辑
- 修复部分非标准 WebDAV 服务器在备份目录已存在时的问题
- 修复 Linux 应用内更新问题
- 修复 JS 脚本验证因 console 方法调用导致的执行失败问题
- 修复 macOS 内存压力下 WebView 渲染进程被系统终止引发的白屏与主进程内存泄漏
- 修复全局热键开关设置可能被意外覆盖的问题
- 修复编辑节点时 Base64 解码未能正确处理非 ASCII 字符的问题
- 修复订阅规则编辑器自定义规则偶发显示为空的问题
- 修复开启“允许局域网连接”后，其他局域网设备仍可能无法连接代理的问题
- 修复“允许局域网连接”页面可能无法显示网络接口的问题
- 修复导入包含无效 v2ray-plugin 参数的 ss:// 链接时可能崩溃的问题
- 修复 macOS 托盘图标可能闪烁的问题
- 修复 macOS 应用启动及从菜单栏重新打开时，窗口可能无法正确激活的问题
- 修复 macOS App Translocation 状态下服务安装或启动异常的问题
- 修复异常窗口尺寸被保存后，应用窗口可能无法正常显示的问题
- 修复首页上传和下载总流量统计不准确的问题
- 修复首页当前代理状态偶尔不同步的问题
- 修复 URLTest 代理组未出现在部分代理组选择列表中的问题
- 修复切换代理模式失败时未正确显示错误的问题
- 修复重新加载 Clash 配置后规则页面数据未及时刷新的问题
- 修复全局快捷键隐藏主窗口行为异常的问题
- 修复外部控制器 CORS 编辑时输入框可能丢失焦点的问题
- 修复日志刷新时页面可能被强制滚动到底部的问题
- 修复全局扩展脚本中的通配符配置可能生成无效 YAML 的问题
- 修复代理组筛选后定位及滚动位置恢复异常的问题
- 修复编辑订阅配置时清空 HTTP 请求超时会报错的问题
- 修复纯 IP 连接的主机地址显示错误，并保留目标 IP 为空时的地址回退
- 修复 Windows 管理员模式下启动 TUN 时因等待不可用 Service 导致延迟 30 秒的问题
- 修复内核启动慢导致首页上下跳动的问题
- 修复 VLESS xhttp 链接解析及 IPv6 地址格式问题
- 修复通过普通导入或深链导入订阅后，自动更新任务未及时生效的问题
- 修复链式配置文件首次编辑失败及校验回滚后遗留空文件的问题，并优化文件读取错误提示

<details>
<summary><strong> ✨ 新增功能 </strong></summary>

- **Mihomo(Meta) 内核升级至 v1.19.29**
- 增加 TrustTunnel、OpenVPN、Tailscale、GostRelay 节点显示支持
- 全局扩展脚本增加恢复默认按钮
- DNS 添加 fake-ip-range6 可配置项
- Linux 无边框窗口支持拖拽调整大小
- 代理组新增筛选、排序、延迟测试和快速定位工具
- 首页当前节点过滤已隐藏的代理组

</details>

<details>
<summary><strong> 🚀 优化改进 </strong></summary>

- 更健壮的服务生命周期管理
- 更健壮的 Mihomo API 通信机制
- 关闭 autofill 弹出窗口
- 改进切换订阅后激活选中节点的逻辑
- 实现代理组粘性滚动列表
- 完善了配置覆写的相关逻辑
- 支持 Provider 节点延迟检测
- 优化页面不可见时的 Mihomo WebSocket 订阅，减少后台资源占用
- 代理组支持分别恢复普通模式和链式代理模式的滚动位置
- 重构改进订阅配置切换逻辑
- 优化大量订阅卡片时的拖拽排序性能

</details>

<details>
<summary><strong> 👙 界面样式 </strong></summary>

- 改进 Sticky Groups 的吸顶阴影、间距和展开/折叠样式
- 优化代理组工具栏和筛选框布局

</details>
