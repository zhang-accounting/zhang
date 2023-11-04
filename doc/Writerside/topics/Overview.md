# zhang

账 (aka zhang) 是一个基于纯文本的复式记账软件，提供了完善的网页和API。zhang 目标在于改进 beancount 的使用体验，所以文法上是类似于 beancount的。


## 特性
- **100% 独立的指令**: 相比于 beancount， zhang 的所有指令都是完全独立的。因此可以以任何的顺序来存储你的账本指令
- **更加精准的时间管理**: zhang 提供了秒级的时间控制，你可以把你的交易、对账等指令都精准到秒
- **更加人性化的文档管理**: zhang 提供了一个更加完善、更加智能的方式来管理交易、账户的相关文档，例如发票、小票等

## beancount 兼容性
beancount 和 zhang 都是基于文本的记账软件，他们都使用着类似的语法。
但是 zhang 废弃了一些指令，例如 `note`, `pad`, `push_tag`。 同理， zhang 也针对了一些指令做出了自己的改进，例如 `balance`。 同时为了兼容 beancount 用户，我们编写 beancount 加载器，可以让 beancount 的用户可以使用 zhang 生态体系里的软件。
