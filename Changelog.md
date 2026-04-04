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
