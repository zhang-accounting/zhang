---
title: 身份认证
description: 用登录页面、密码和通行密钥保护你的张记账实例。
sidebar:
  order: 5
---

在没有任何配置的情况下，任何能访问到张记账的人都可以读取和修改账本。当实例可以被他人访问时（例如部署在服务器上），请至少启用一种登录方式。

有两种方式可用，并且可以同时启用：

| `ZHANG_AUTH` | `ZHANG_PASSKEY` | 登录页面显示 |
| --- | --- | --- |
| 未设置 | 未设置 | 没有登录页面，直接打开网页界面 |
| 已设置 | 未设置 | 用户名和密码表单 |
| 未设置 | 已设置 | **使用通行密钥登录**按钮 |
| 已设置 | 已设置 | 两者都有 |

启用某种方式后，打开网页界面时会先显示登录页面。登录后，浏览器会保存一个会话 cookie，因此在会话过期或你选择**退出登录**之前都会保持登录状态。

## 密码

密码方式使用用户名和密码，格式为 `{USERNAME}:{PASSWORD}`，例如 `admin:admin888`。密码可以包含冒号，第一个冒号用来分隔用户名和密码。

用命令行参数 `--auth` 启用它：

```shell
zhang serve /path/to/ledger --auth admin:admin888
```

或者用环境变量 `ZHANG_AUTH`，这更适合 Docker：

```shell
docker run --name zhang -e "ZHANG_AUTH=admin:admin888" kilerd/zhang:latest
```

:::note
命令行参数优先于环境变量：两者都提供时，使用命令行参数。不含冒号的值会被忽略，密码方式保持关闭。
:::

用户名和密码在登录页面中输入，而不是在浏览器弹窗中。修改 `ZHANG_AUTH` 后，所有用旧凭据创建的会话都会退出登录。

### 脚本和其他 HTTP 客户端

不是浏览器的客户端可以继续在每个请求中以 HTTP Basic `Authorization` 请求头发送凭据：

```shell
curl -u admin:admin888 http://localhost:8000/api/info
```

对 `/api/*` 的请求如果没有有效的会话或 `Authorization` 请求头，会得到 `401` 响应，JSON 响应体为 `{"message": "unauthorized"}`。

## 通行密钥

通行密钥让你用面容 ID、触控 ID、Windows Hello、手机或安全密钥登录，而不必使用密码。用命令行参数 `--passkey` 或环境变量 `ZHANG_PASSKEY` 启用它：

```shell
docker run --name zhang -e "ZHANG_PASSKEY=a-long-random-secret" kilerd/zhang:latest
```

`ZHANG_PASSKEY` 的值是**注册密钥**：任何知道它的人都可以在未登录的情况下注册通行密钥。请选择一个足够长的随机值并妥善保密。登录时不需要它。

### 注册第一个通行密钥

1. 设置好 `ZHANG_PASSKEY` 后启动张记账，并打开网页界面。
2. 由于还没有注册任何通行密钥，登录页面会提示你**设置通行密钥**。
3. 输入注册密钥，并可选地为通行密钥起一个名字。
4. 在设备上确认。你会随即登录，下次使用**使用通行密钥登录**即可。

### 添加更多通行密钥

登录后，打开**设置 → 通行密钥**并选择**添加通行密钥**，即可在当前设备上创建通行密钥，无需注册密钥。要在另一台设备上首次登录，可以使用会同步到该设备的通行密钥（例如通过 iCloud 钥匙串或 Google 密码管理工具），通过浏览器的“使用其他设备”选项借助手机登录，或者在启用了密码方式时用密码登录，然后在那台设备上添加通行密钥。

### 移除通行密钥

在**设置 → 通行密钥**中移除通行密钥。用它登录的会话会立即结束。密码方式关闭时，最后一个通行密钥无法移除，以免你被锁在门外。

从账本中移除通行密钥并不会把它从你的设备或密码管理器中删除，如果不再需要，也请在那里删除。

### 通行密钥的存储位置

已注册的通行密钥保存在账本根目录的 `.zhang/passkeys.json` 中，通过所配置的数据源（本地文件、S3、WebDAV 或 GitHub）存储，因此重启和重新部署后依然有效。这个文件只包含公钥，但请把它和账本放在一起，并纳入备份。如果它被删除，请用注册密钥重新注册通行密钥。

修改 `ZHANG_PASSKEY` 不会移除已经注册的通行密钥。如果注册密钥泄露，请修改它，并检查**设置 → 通行密钥**中的列表。

### 域名与反向代理

浏览器只允许在通过 HTTPS 访问的域名上或在 `localhost` 上使用通行密钥。通过 `http://192.168.1.10:8000` 这样的 IP 地址打开网页界面时，通行密钥无法使用。

通行密钥绑定在它的**依赖方 ID**（relying party ID）上，也就是创建它时的域名。默认情况下，张记账使用浏览器看到的请求主机：[反向代理](#反向代理)的 `X-Forwarded-Host` 和 `X-Forwarded-Proto` 请求头优先于 `Host` 请求头。大多数代理和托管平台都会设置它们，所以自定义域名通常无需任何配置即可使用。

如果代理不转发这些请求头，或者要在子域名之间共享通行密钥，请显式设置：

- `ZHANG_PASSKEY_ORIGIN`：打开网页界面所用的地址，例如 `https://zhang.example.com`。设置后，依赖方 ID 默认为它的主机。
- `ZHANG_PASSKEY_RP_ID`：依赖方 ID，例如 `example.com`。它必须是 origin 的主机或其上级域名之一。

```shell
docker run --name zhang \
  -e "ZHANG_PASSKEY=a-long-random-secret" \
  -e "ZHANG_PASSKEY_ORIGIN=https://zhang.example.com" \
  kilerd/zhang:latest
```

为一个域名创建的通行密钥不能在另一个域名上使用：把实例迁移到新域名后，请用密码登录，或者用注册密钥重新注册通行密钥。

## 会话

登录会设置一个名为 `zhang_session` 的 cookie，有效期 30 天。它带有 `HttpOnly` 和 `SameSite=Lax`，当浏览器通过 HTTPS 访问服务器时（以[反向代理](#反向代理)的 `X-Forwarded-Proto` 请求头为准）还会标记为 `Secure`。

会话用一个密钥签名。用环境变量 `ZHANG_SESSION_SECRET` 设置它，就能在重启和重新部署后让所有人保持登录：

```shell
docker run --name zhang \
  -e "ZHANG_AUTH=admin:admin888" \
  -e "ZHANG_SESSION_SECRET=$(openssl rand -hex 32)" \
  kilerd/zhang:latest
```

没有 `ZHANG_SESSION_SECRET` 时，服务器每次启动都会生成一个随机密钥，所以重启会让所有浏览器退出登录。修改这个密钥同样会让所有浏览器退出登录。

## 登录失败次数限制

为了减缓密码猜测，张记账会统计失败的尝试：密码登录失败，以及输错通行密钥的注册密钥。同一地址在 15 分钟内失败 5 次，或者所有地址合计失败 50 次之后，后续尝试都会被拒绝，返回 `429 Too Many Requests`（并带有 `Retry-After` 请求头），直到 15 分钟过去，即使密码正确也一样。成功登录会重置该地址的计数。已经登录的浏览器和通行密钥登录不受影响。

地址取自[反向代理](#反向代理)在 `X-Forwarded-For` 中报告的地址，没有代理时则是连接的地址。计数保存在内存中，服务器重启后重新开始。

## 反向代理

在反向代理之后，张记账从 `X-Forwarded-For`、`X-Forwarded-Host` 和 `X-Forwarded-Proto` 请求头的最右一项获取浏览器的地址，以及它所打开的主机和协议：最右一项是代理追加的，之前的各项则是浏览器自己发送的任意内容。这在单层代理之后是正确的，例如 Railway 的边缘节点，所以在那里不需要任何配置。在多层代理之后，或者通行密钥看到的主机或协议不正确时，请设置 `ZHANG_PASSKEY_ORIGIN`（以及 `ZHANG_PASSKEY_RP_ID`），见[域名与反向代理](#域名与反向代理)。

## 故障排除

- **网页界面打开时没有登录页面**：`ZHANG_AUTH` 和 `ZHANG_PASSKEY` 都没有传到服务器。使用 Docker 时，请检查 `-e` 选项；`ZHANG_AUTH` 需要使用 `{USERNAME}:{PASSWORD}` 格式。
- **“用户名或密码不正确。”**：检查 `ZHANG_AUTH` 的值，用户名和密码都区分大小写。
- **每次重启后都会退出登录**：设置 `ZHANG_SESSION_SECRET`。
- **登录后马上又回到登录页面**：浏览器没有保存会话 cookie。在设置了 `X-Forwarded-Proto: https` 的代理之后，cookie 带有 `Secure`，只会在 HTTPS 下保存；请通过 HTTPS 打开网页界面。
- **没有出现通行密钥选项，或者浏览器拒绝使用**：通过域名（或 `localhost`）以 HTTPS 打开网页界面；如果它运行在不转发主机的代理之后，请设置 `ZHANG_PASSKEY_ORIGIN`。
- **“注册密钥不正确。”**：输入服务器启动时所用的 `ZHANG_PASSKEY` 的准确值。
- **“too many attempts, try again in N minutes”**：失败的尝试太多，见[登录失败次数限制](#登录失败次数限制)。请等待，或者重启服务器以清除计数。
