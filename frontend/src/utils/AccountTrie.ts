import { AccountListItem } from '@/api/types';

/** The account tree by the parts of the names: its structure only; the value of a node is `treeTotals`'s (account-totals.ts). */
export default class AccountTrie {
  children: { [layer: string]: AccountTrie } = {};
  val?: AccountListItem;
  word?: string;
  path: string = '';
  isLeaf?: boolean | undefined = true;

  insert(account: AccountListItem) {
    // eslint-disable-next-line @typescript-eslint/no-this-alias -- walking the trie from the root node
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
    node.isLeaf = true;
    node.word = word;
    node.val = account;
  }
}
