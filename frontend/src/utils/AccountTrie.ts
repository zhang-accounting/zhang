import { Account } from '../rest-model';
import { BigNumber } from 'bignumber.js';

export default class AccountTrie {
  children: { [layer: string]: AccountTrie } = {};
  val?: Account;
  word?: string;
  path: string = '';
  isLeaf?: boolean | undefined = true;
  amount: MultiCommodityAmount = new MultiCommodityAmount();

  insert(account: Account) {
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
      for (const [commodity, amount] of Object.entries(account.commodities)) {
        node.amount.insert(amount, commodity);
      }
      node = node.children[ch];
    }
    for (const [commodity, amount] of Object.entries(account.commodities)) {
      node.amount.insert(amount, commodity);
    }
    node.isLeaf = true;
    node.word = word;
    node.val = account;
  }
}

export class MultiCommodityAmount {
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

  merge(other: MultiCommodityAmount) {
    Object.keys(other.data).forEach((commodity) => {
      this.insertBigNumber(other.data[commodity], commodity);
    });
  }
}
