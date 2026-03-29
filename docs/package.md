# 打包

## 打标

```bash
# 触发 GitHub action 自动发布新包
TAG=v2.4.100107 && git tag $TAG && git push origin $TAG
```

## 签名问题

> 需要 Apple Developer 账号（$99/年）进行代码签名和公证（notarize）。没有这个，macOS 14+ 会越来越严格地拦截未签名应用。
> 中间方案 — DMG 签名（无需开发者账号，但效果有限）：
> 可以用自签名证书签名，能通过部分检查但不能过 Gatekeeper 完整验证，用户仍需要手动 xattr -cr。

```bash
# 用户侧临时解决方案（无需证书）：
xattr -cr "/Applications/Clash Verge.app"
```
