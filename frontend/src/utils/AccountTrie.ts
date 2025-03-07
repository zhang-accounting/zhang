import { Account, AccountListItem } from '@/api/types';
import { BigNumber } from 'bignumber.js';

export default class AccountTrie {
  children: { [layer: string]: AccountTrie } = {};
  val?: AccountListItem;
  word?: string;
  path: string = '';
  isLeaf?: boolean | undefined = true;
  amount: MultiCommodityAmount = new MultiCommodityAmount();

  insert(account: AccountListItem) {
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
      node.amount.merge(account.amount);
      node = node.children[ch];
    }
    node.amount.merge(account.amount);
    node.isLeaf = true;
    node.word = word;
    node.val = account;
  }
}

export class MultiCommodityAmount {
  total: BigNumber = new BigNumber(0);
  commodity: string = '';
  data: { [commodity: string]: BigNumber } = {};

  insert(amount: string, commodity: string) {
    this.insertBigNumber(new BigNumber(amount), commodity);
  }
  insertBigNumber(amount: BigNumber, commodity: string) {
    if (!this.data[commodity]) {
      this.data[commodity] = new BigNumber(0);
    }
    this.data[commodity] = this.data[commodity].plus(amount);
  }
  merge(other: Account["amount"]) {
    this.total = this.total.plus(other.calculated.number);
    this.commodity = other.calculated.currency;
    Object.keys(other.detail).forEach((commodity) => {
      this.insert(other.detail[commodity], commodity);
    });
  }
}
