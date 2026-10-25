# Git 推送协议约定：一律使用 SSH

> 生效日期：2026-10-25
> 适用范围：本仓库（AI_comtool-rs / Com-tool_RS）
> 作者确认：GitHub 公钥已由用户配置完成，并已实测可认证。

## 约定

- **所有 `git push` 一律走 SSH 协议，不再使用 HTTPS。**
- 本约定针对 git 传输层；与 GitHub REST API 认证无关（见下方注意事项）。

## 当前 remote（已改）

```
origin  git@github.com:Tesla1983/Com-tool_RS.git (fetch)
origin  git@github.com:Tesla1983/Com-tool_RS.git (push)
```

改之前的地址（仅作记录，不应再使用）：

```
https://github.com/Tesla1983/Com-tool_RS.git
```

## 验证

- 连接测试（非交互，适合脚本/CI 自检）：

  ```bash
  ssh -o StrictHostKeyChecking=accept-new -o BatchMode=yes -T git@github.com
  ```

  成功回显：

  ```
  Hi Tesla1983! You've successfully authenticated, but GitHub does not provide shell access.
  ```

  ⚠️ GitHub 对 `ssh -T` 故意返回**非 0 退出码**（不提供 shell 访问），
  **以回显中的 `successfully authenticated` 为准，不要以 exit code 判成败。**

- 正常推送：`git push origin <branch>`

## 注意事项

1. **SSH key 只对 git 传输有效，与 GitHub REST API 认证无关。**
   查 CI / actions 状态、读仓库元数据仍需 token（`gh` CLI 或 `Invoke-RestMethod` + PAT），
   **不能**用 SSH 免密替代。这条与本约定互不冲突、各自独立。
2. 换机器后：需重新把公钥加入 GitHub 账户，并 `ssh -T git@github.com` 复测。
3. 若 remote 被意外改回 HTTPS，恢复命令：

   ```bash
   git remote set-url origin git@github.com:Tesla1983/Com-tool_RS.git
   ```

## 全局代替规则（已生效：2026-10-25）

本机**所有 GitHub 仓库**已统一走 SSH，不再用 HTTPS。已写入全局 git 配置：

```bash
git config --global url."git@github.com:".insteadOf "https://github.com/"
```

生效后：任何 `https://github.com/...` 形式的 URL 在 git 传输时自动改写为
`git@github.com:...`（SSH）。验证：

```bash
git config --global --get-regexp 'url\..*\.insteadOf'
# 期望输出：url.git@github.com:.insteadof https://github.com/
```

⚠️ 注意：此规则只影响 **git 传输**。若某仓库的 remote 当前已显式写成 SSH
（`git@github.com:...`），代替规则不会重复改写，无害。

若需撤销全局规则：

```bash
git config --global --unset url."git@github.com:".insteadOf
```
