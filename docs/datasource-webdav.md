# WebDav 数据源

下面是启动 webdav 所需要的参数配置：

| 名称           | 命令行参数  | 环境变量                  | 必填      | 值       | 备注                                  |
|--------------|--------|-----------------------|---------|---------|-------------------------------------|
| 数据源          | source | ZHANG_DATA_SOURCE     | 是 | `web-dav` |
| webdav 服务器地址 |        | ZHANG_WEBDAV_ENDPOINT | 是       |         | 例如：`https://dav.jianguoyun.com/dav` |
| webdav 数据根目录 |        | ZHANG_WEBDAV_ROOT     | 是       |         | 例如： `/accounting`                   |
| webdav 用户名   |        | ZHANG_WEBDAV_USERNAME | 是       |         |                                     |
| webdav 密码    |        | ZHANG_WEBDAV_PASSWORD | 否       |         |                                     |