---
title: 在 Render 部署在线 demo
description: 使用 Render 免费实例和只读 S3 或 Cloudflare R2 存储桶，不通过 Docker 部署张记账。
---

仓库里的 `render.yaml` 把前端嵌入 Rust 二进制，并通过 Render 的原生 Rust 环境运行 **Free** 网页服务。账本保存在兼容 S3 的存储中，Render 不需要持久磁盘或数据库。

## 准备只读存储

1. 使用专门的存储桶，只放可以公开的 demo 数据。首次部署可以将 `examples/main.zhang` 上传为 `main.zhang`。如果账本包含其他文件、附件或插件模块，一并上传并保持相对路径。
2. 给服务使用的凭据只授予该存储桶的**读取和列举**权限，不授予写入或删除权限。上传和维护示例数据的凭据单独保管，不提供给 Render。
   - **Cloudflare R2：**创建权限为 **Object Read only**、限定到 demo 存储桶的 S3 API token。使用生成的 Access Key ID 和 Secret Access Key，S3 endpoint 为 `https://<account_id>.r2.cloudflarestorage.com`，region 为 `auto`。有管辖区域限制的存储桶要使用对应区域的 endpoint。桶可以保持私有，访客通过张记账读取内容。参见 [R2 鉴权文档](https://developers.cloudflare.com/r2/api/tokens/)。
   - **Amazon S3：**使用独立的 IAM 身份，只授予 demo 对象的 `s3:GetObject` 和 demo 桶的 `s3:ListBucket`。如果使用自管 KMS 密钥加密账本，还需要该密钥的解密权限。region 和 S3 endpoint 使用存储桶实际所在区域。参见 [S3 策略示例](https://docs.aws.amazon.com/AmazonS3/latest/userguide/example-policies-s3.html)。

即使访客直接向 API 发送写入请求，只读凭据也会保护源文件。它**不会隐藏网页上的编辑控件**：保存文件、新建或修改交易、上传附件、注册通行密钥都会因存储拒绝写入而失败。浏览、查询和 CSV 导出可以正常使用。匿名 demo 不启用密码或通行密钥登录。

## 部署 Blueprint

1. 把部署文件推送到 GitHub 分支。可以直接使用 PR 分支部署 demo，无需先合入。
2. 在 [Render 面板](https://dashboard.render.com) 选择 **New → Blueprint**，连接仓库并选择该分支，Blueprint 路径保留为 `render.yaml`。
3. 填写 Render 提示的环境变量：

   | 变量 | 值 |
   | --- | --- |
   | `ZHANG_S3_BUCKET` | demo 存储桶名。 |
   | `ZHANG_S3_ENDPOINT` | S3 API 地址，不是公开下载地址。 |
   | `ZHANG_S3_REGION` | R2 填 `auto`；AWS S3 填桶的实际区域。 |
   | `ZHANG_S3_ACCESS_KEY_ID` | 只读 Access Key ID。 |
   | `ZHANG_S3_SECRET_ACCESS_KEY` | 只读 Secret Access Key。 |

4. 检查服务为 **Rust**、**Free**、Singapore，没有磁盘或数据库，然后部署。
5. 账本不在桶根目录时，将 `ZHANG_S3_ROOT` 设为对应前缀（例如 `/accounting`）。主文件不是 `main.zhang` 时，修改 `ZHANG_DEMO_ENDPOINT`。在部署前保存这些设置，或修改后重新部署。入口也可以是 `.bean` 文件。使用临时 AWS 凭据时，另加 `ZHANG_S3_SESSION_TOKEN` 并在过期前更新凭据。
6. 打开服务的 `onrender.com` 地址，确认预期账户和交易出现，并运行一次 Explore 查询、导出 CSV。验证只读权限时，用 **demo 凭据**尝试写入一个独立的临时对象，确认被存储服务拒绝。

构建脚本固定使用 pnpm 9，先构建前端，再仅编译启用了前端嵌入功能的 `zhang` 二进制。启动脚本检查必填设置并监听 `0.0.0.0:$PORT`。健康检查使用 `/api/info`；还需检查账户数据，因为找不到主文件时，空账本也可能通过健康检查。

配置关闭了自动部署，避免每次提交都消耗构建分钟。更新分支后使用 **Manual Deploy**。桶内文件更新后，使用张记账的重新加载按钮或重启服务：张记账不会监视远程存储。Render 重新部署或重启不会影响桶里的源文件，本地附件和插件缓存可重新生成。

## 免费额度

Render 免费实例在 15 分钟没有入站流量后休眠，再次访问通常需要约一分钟启动。每个 workspace 每月有 750 小时免费实例时间，由该 workspace 的免费服务共享；构建分钟和出站流量也有额度限制。没有绑定付款方式时，额度用尽会暂停服务或停止新构建，而不是收取超额费用。如果账号已绑定付款方式，要单独检查账单和支出设置：`plan: free` 本身不会关闭所有用量收费。参见 [Render 免费版限制](https://render.com/docs/free)。

R2 Standard 存储每月包含 10 GB-month 存储、100 万次 Class A 操作、1000 万次 Class B 操作的免费额度，直接从 R2 出站的流量免费。超过额度仍会计费，所以只读权限不等于无限免费存储或请求。参见 [R2 价格](https://developers.cloudflare.com/r2/pricing/)。
