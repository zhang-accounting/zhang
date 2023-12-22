# Web 基本认证

zhang 提供了自带的basic auth 支持，basic auth 要求用户在启动zhang时提供明文的用户名与密码，格式遵循`{USERNAME}:{PASSWORD}` 的格式。例子：
 - `username:password`
 - `admin:admin888`

你可以通过下面两种方式启动basic auth： 

**命令行参数`auth`**:

`docker run --name zhang  kilerd/zhang:snapshot --auth admin:admin888`

**环境变量 `ZHANG_AUTH`**: 

`docker run --name zhang -e "ZHANG_AUTH=admin:admin888" kilerd/zhang:snapshot`

!> **命令行参数**具有更高的优先级，当两个配置参数都提供时，会优先采用**命令行参数**