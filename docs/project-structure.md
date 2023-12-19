# 项目、组件

## AST

## 核心 Core

### 数据源与结构
zhang 的设计目标是不依赖，也不强绑定任何格式， 也就是说你可以把数据存储在**数据库**，**文本**，**二进制**，以及任意可以自由表达的地方。

为了可以让 zhang 顺利地工作，我们抽象出了两个组件用来帮助如何处理不同的数据源与结构。
### DataSource
DataSource 指的是数据源，简单来说，你的数据存储在何处，本地文件系统呢？远端 GitHub 呢？，所以DataSource 告诉了 zhang 核心
 - 如何读取数据，并处理成标准的、zhang可以理解的 `Directive`
 - 如何把一个 `Directive` 写回数据源，通常用于数据更新

一旦数据源决定了，那意味着会跟一个数据类型 `DataType` 做绑定，所以在构建 `DataSource` 的时候需要指明具体的数据类型。

### DateType

DataType 表示着源数据是采用哪一种结构存储，比如纯文本，json，数据库 等等。 `DataType`的核心就是把标准的 `Directive` 转换成对应的储存格式，zhang 官方维护了几种数据格式：
- `zhang` 改进 beancount 纯文本格式
- `beancount` beancount 官方纯文本格式