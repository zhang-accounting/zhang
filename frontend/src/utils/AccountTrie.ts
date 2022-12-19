import {Account} from "../rest-model";

export default class AccountTrie {
  children: { [layer: string]: AccountTrie } = {};
  val?: Account;
  word?: string;
  path: string = '';
  isLeaf?: boolean | undefined = true;

  insert(account: any) {
    let node: AccountTrie = this;
    let word: string = '';
    for (const ch of account.name.split(':')) {
      if (!node.children[ch]) {
        node.children[ch] = new AccountTrie();

        word = ch;
        node.children[ch].word = word;
        node.children[ch].path = [node.path, ch].filter((item) => item.length > 0).join(':');
        node.isLeaf = false;
      }
      node = node.children[ch];
    }
    node.word = word;
    node.val = account;
  }
}
