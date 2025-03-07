/**
 * This file was auto-generated by openapi-typescript.
 * Do not make direct changes to the file.
 */

export interface paths {
  '/api/accounts': {
    /** Get Account List */
    get: operations['get_account_list'];
  };
  '/api/accounts/:account_name': {
    /** Get Account Info */
    get: operations['get_account_info'];
  };
  '/api/accounts/:account_name/balances': {
    /** Get Account Balance Data */
    get: operations['get_account_balance_data'];
    /** Create Account Balance */
    post: operations['create_account_balance'];
  };
  '/api/accounts/:account_name/documents': {
    /** Get Account Documents */
    get: operations['get_account_documents'];
    /** Upload Account Document */
    post: operations['upload_account_document'];
  };
  '/api/accounts/:account_name/journals': {
    /** Get Account Journals */
    get: operations['get_account_journals'];
  };
  '/api/accounts/batch-balances': {
    /** Create Batch Account Balances */
    post: operations['create_batch_account_balances'];
  };
  '/api/budgets': {
    /** Get Budget List */
    get: operations['get_budget_list'];
  };
  '/api/budgets/:budget_name': {
    /** Get Budget Info */
    get: operations['get_budget_info'];
  };
  '/api/budgets/:budget_name/interval/:year/:month': {
    /** Get Budget Interval Detail */
    get: operations['get_budget_interval_detail'];
  };
  '/api/commodities': {
    /** Get All Commodities */
    get: operations['get_all_commodities'];
  };
  '/api/commodities/:commodity_name': {
    /** Get Single Commodity */
    get: operations['get_single_commodity'];
  };
  '/api/documents': {
    /** Get Documents */
    get: operations['get_documents'];
  };
  '/api/errors': {
    /** Get Errors */
    get: operations['get_errors'];
  };
  '/api/files': {
    /** Get Files */
    get: operations['get_files'];
  };
  '/api/files/:file_path': {
    /** Get File Content */
    get: operations['get_file_content'];
    /** Update File Content */
    put: operations['update_file_content'];
  };
  '/api/for-new-transaction': {
    /** Get Info For New Transactions */
    get: operations['get_info_for_new_transactions'];
  };
  '/api/info': {
    /** Get Basic Info */
    get: operations['get_basic_info'];
  };
  '/api/journals': {
    /** Get Journals */
    get: operations['get_journals'];
  };
  '/api/options': {
    /** Get All Options */
    get: operations['get_all_options'];
  };
  '/api/plugins': {
    /** Plugin List */
    get: operations['plugin_list'];
  };
  '/api/reload': {
    /** Reload */
    post: operations['reload'];
  };
  '/api/statistic/:account_type': {
    /** Get Statistic Rank Detail By Account Type */
    get: operations['get_statistic_rank_detail_by_account_type'];
  };
  '/api/statistic/graph': {
    /** Get Statistic Graph */
    get: operations['get_statistic_graph'];
  };
  '/api/statistic/summary': {
    /** Get Statistic Summary */
    get: operations['get_statistic_summary'];
  };
  '/api/transactions': {
    /** Create New Transaction */
    post: operations['create_new_transaction'];
  };
  '/api/transactions/:transaction_id': {
    /** Update Single Transaction */
    put: operations['update_single_transaction'];
  };
  '/api/transactions/:transaction_id/documents': {
    /** Upload Transaction Document */
    post: operations['upload_transaction_document'];
  };
}

export type webhooks = Record<string, never>;

export type components = Record<string, never>;

export type $defs = Record<string, never>;

export type external = Record<string, never>;

export interface operations {
  /** Get Account List */
  get_account_list: {
    responses: {
      /** @description default return */
      200: {
        content: {
          'application/json': {
            data: {
              alias?: string | null;
              amount: {
                /** @description the calculated amount */
                calculated: {
                  /** @description the currency of the amount */
                  currency: string;
                  /** @description the number of the amount */
                  number: string;
                };
                /** @description the detail of the calculated amount */
                detail: {
                  [key: string]: string;
                };
              };
              name: string;
              /** @enum {string} */
              status: 'Open' | 'Close';
            }[];
          };
        };
      };
    };
  };
  /** Get Account Info */
  get_account_info: {
    parameters: {
      path: {
        account_name: string;
      };
    };
    responses: {
      /** @description default return */
      200: {
        content: {
          'application/json': {
            data: {
              alias?: string | null;
              amount: {
                /** @description the calculated amount */
                calculated: {
                  /** @description the currency of the amount */
                  currency: string;
                  /** @description the number of the amount */
                  number: string;
                };
                /** @description the detail of the calculated amount */
                detail: {
                  [key: string]: string;
                };
              };
              /** Format: date-time */
              date: string;
              name: string;
              'r#type': string;
              /** @enum {string} */
              status: 'Open' | 'Close';
            };
          };
        };
      };
    };
  };
  /** Get Account Balance Data */
  get_account_balance_data: {
    parameters: {
      path: {
        account_name: string;
      };
    };
    responses: {
      /** @description default return */
      200: {
        content: {
          'application/json': {
            data: {
              balance: {
                [key: string]: {
                  balance: {
                    commodity: string;
                    number: string;
                  };
                  /** Format: date */
                  date: string;
                }[];
              };
            };
          };
        };
      };
    };
  };
  /** Create Account Balance */
  create_account_balance: {
    parameters: {
      path: {
        account_name: string;
      };
    };
    requestBody: {
      content: {
        'application/json':
          | {
              account_name: string;
              amount: {
                commodity: string;
                number: string;
              };
              /** @enum {string} */
              type: 'Check';
            }
          | {
              account_name: string;
              amount: {
                commodity: string;
                number: string;
              };
              pad: string;
              /** @enum {string} */
              type: 'Pad';
            };
      };
    };
    responses: {
      /** @description no content */
      204: {
        content: never;
      };
    };
  };
  /** Get Account Documents */
  get_account_documents: {
    parameters: {
      path: {
        account_name: string;
      };
    };
    responses: {
      /** @description default return */
      200: {
        content: {
          'application/json': {
            data: {
              account?: string | null;
              /** Format: date-time */
              datetime: string;
              extension?: string | null;
              filename: string;
              path: string;
              trx_id?: string | null;
            }[];
          };
        };
      };
    };
  };
  /** Upload Account Document */
  upload_account_document: {
    parameters: {
      path: {
        account_name: string;
      };
    };
    /** @description Multipart form data */
    requestBody: {
      content: {
        'multipart/form-data': unknown;
      };
    };
    responses: {
      /** @description no content */
      204: {
        content: never;
      };
    };
  };
  /** Get Account Journals */
  get_account_journals: {
    parameters: {
      path: {
        account_name: string;
      };
    };
    responses: {
      /** @description default return */
      200: {
        content: {
          'application/json': {
            data: {
              account: string;
              account_after_commodity: string;
              account_after_number: string;
              /** Format: date-time */
              datetime: string;
              inferred_unit_commodity: string;
              inferred_unit_number: string;
              narration?: string | null;
              payee?: string | null;
              timestamp: number;
              trx_id: string;
            }[];
          };
        };
      };
    };
  };
  /** Create Batch Account Balances */
  create_batch_account_balances: {
    requestBody: {
      content: {
        'application/json': (
          | {
              account_name: string;
              amount: {
                commodity: string;
                number: string;
              };
              /** @enum {string} */
              type: 'Check';
            }
          | {
              account_name: string;
              amount: {
                commodity: string;
                number: string;
              };
              pad: string;
              /** @enum {string} */
              type: 'Pad';
            }
        )[];
      };
    };
    responses: {
      /** @description no content */
      204: {
        content: never;
      };
    };
  };
  /** Get Budget List */
  get_budget_list: {
    parameters: {
      query?: {
        month?: number | null;
        year?: number | null;
      };
    };
    responses: {
      /** @description default return */
      200: {
        content: {
          'application/json': {
            data: {
              activity_amount: {
                /** @description the currency of the amount */
                currency: string;
                /** @description the number of the amount */
                number: string;
              };
              alias?: string | null;
              assigned_amount: {
                /** @description the currency of the amount */
                currency: string;
                /** @description the number of the amount */
                number: string;
              };
              available_amount: {
                /** @description the currency of the amount */
                currency: string;
                /** @description the number of the amount */
                number: string;
              };
              category?: string | null;
              closed: boolean;
              name: string;
            }[];
          };
        };
      };
    };
  };
  /** Get Budget Info */
  get_budget_info: {
    parameters: {
      query?: {
        month?: number | null;
        year?: number | null;
      };
      path: {
        budget_name: string;
      };
    };
    responses: {
      /** @description default return */
      200: {
        content: {
          'application/json': {
            data: {
              activity_amount: {
                /** @description the currency of the amount */
                currency: string;
                /** @description the number of the amount */
                number: string;
              };
              alias?: string | null;
              assigned_amount: {
                /** @description the currency of the amount */
                currency: string;
                /** @description the number of the amount */
                number: string;
              };
              available_amount: {
                /** @description the currency of the amount */
                currency: string;
                /** @description the number of the amount */
                number: string;
              };
              category?: string | null;
              closed: boolean;
              name: string;
              related_accounts: string[];
            };
          };
        };
      };
    };
  };
  /** Get Budget Interval Detail */
  get_budget_interval_detail: {
    parameters: {
      path: {
        budget_name: string;
        month: number;
        year: number;
      };
    };
    responses: {
      /** @description default return */
      200: {
        content: {
          'application/json': {
            data: (
              | {
                  amount: {
                    /** @description the currency of the amount */
                    currency: string;
                    /** @description the number of the amount */
                    number: string;
                  };
                  /** @enum {string} */
                  event_type: 'AddAssignedAmount' | 'Transfer';
                  timestamp: number;
                  /** @enum {string} */
                  type: 'BudgetEvent';
                }
              | {
                  account: string;
                  account_after_commodity: string;
                  account_after_number: string;
                  /** Format: date-time */
                  datetime: string;
                  inferred_unit_commodity: string;
                  inferred_unit_number: string;
                  narration?: string | null;
                  payee?: string | null;
                  timestamp: number;
                  trx_id: string;
                  /** @enum {string} */
                  type: 'Posting';
                }
            )[];
          };
        };
      };
    };
  };
  /** Get All Commodities */
  get_all_commodities: {
    responses: {
      /** @description default return */
      200: {
        content: {
          'application/json': {
            data: {
              group?: string | null;
              latest_price_amount?: string | null;
              latest_price_commodity?: string | null;
              /** Format: date-time */
              latest_price_date?: string | null;
              name: string;
              precision: number;
              prefix?: string | null;
              rounding: string;
              suffix?: string | null;
              total_amount: string;
            }[];
          };
        };
      };
    };
  };
  /** Get Single Commodity */
  get_single_commodity: {
    parameters: {
      path: {
        commodity_name: string;
      };
    };
    responses: {
      /** @description default return */
      200: {
        content: {
          'application/json': {
            data: {
              info: {
                group?: string | null;
                latest_price_amount?: string | null;
                latest_price_commodity?: string | null;
                /** Format: date-time */
                latest_price_date?: string | null;
                name: string;
                precision: number;
                prefix?: string | null;
                rounding: string;
                suffix?: string | null;
                total_amount: string;
              };
              lots: {
                account: string;
                /** Format: date */
                acquisition_date?: string | null;
                amount: string;
                cost?: {
                  /** @description the currency of the amount */
                  currency: string;
                  /** @description the number of the amount */
                  number: string;
                } | null;
                price?: {
                  /** @description the currency of the amount */
                  currency: string;
                  /** @description the number of the amount */
                  number: string;
                } | null;
              }[];
              prices: {
                amount: string;
                /** Format: date-time */
                datetime: string;
                target_commodity?: string | null;
              }[];
            };
          };
        };
      };
    };
  };
  /** Get Documents */
  get_documents: {
    responses: {
      /** @description default return */
      200: {
        content: {
          'application/json': {
            data: {
              account?: string | null;
              /** Format: date-time */
              datetime: string;
              extension?: string | null;
              filename: string;
              path: string;
              trx_id?: string | null;
            }[];
          };
        };
      };
    };
  };
  /** Get Errors */
  get_errors: {
    parameters: {
      query?: {
        page?: number | null;
        size?: number | null;
        keyword?: string | null;
        tags?: string[] | null;
        links?: string[] | null;
      };
    };
    responses: {
      /** @description default return */
      200: {
        content: {
          'application/json': {
            data: {
              current_page: number;
              page_size: number;
              records: {
                /** @enum {string} */
                error_type:
                  | 'UnbalancedTransaction'
                  | 'TransactionCannotInferTradeAmount'
                  | 'TransactionHasMultipleImplicitPosting'
                  | 'TransactionExplicitPostingHaveMultipleCommodity'
                  | 'AccountBalanceCheckError'
                  | 'AccountDoesNotExist'
                  | 'AccountClosed'
                  | 'CommodityDoesNotDefine'
                  | 'NoEnoughCommodityLot'
                  | 'CloseNonZeroAccount'
                  | 'BudgetDoesNotExist'
                  | 'DefineDuplicatedBudget'
                  | 'MultipleOperatingCurrencyDetect'
                  | 'ParseInvalidMeta';
                id: string;
                metas: {
                  [key: string]: string;
                };
                span?: {
                  content: string;
                  end: number;
                  filename?: string | null;
                  start: number;
                } | null;
              }[];
              total_count: number;
              total_page: number;
            };
          };
        };
      };
    };
  };
  /** Get Files */
  get_files: {
    responses: {
      /** @description default return */
      200: {
        content: {
          'application/json': {
            data?: (string | null)[];
          };
        };
      };
    };
  };
  /** Get File Content */
  get_file_content: {
    parameters: {
      path: {
        file_path: string;
      };
    };
    responses: {
      /** @description default return */
      200: {
        content: {
          'application/json': {
            data: {
              content: string;
              path: string;
            };
          };
        };
      };
    };
  };
  /** Update File Content */
  update_file_content: {
    parameters: {
      path: {
        file_path: string;
      };
    };
    requestBody: {
      content: {
        'application/json': {
          content: string;
        };
      };
    };
    responses: {
      /** @description no content */
      204: {
        content: never;
      };
    };
  };
  /** Get Info For New Transactions */
  get_info_for_new_transactions: {
    responses: {
      /** @description default return */
      200: {
        content: {
          'application/json': {
            data: {
              account_name: string[];
              payee: string[];
            };
          };
        };
      };
    };
  };
  /** Get Basic Info */
  get_basic_info: {
    responses: {
      /** @description default return */
      200: {
        content: {
          'application/json': {
            data: {
              /** @description docker build date of zhang accounting */
              build_date: string;
              /** @description title of ledger */
              title?: string | null;
              /** @description version of zhang accounting */
              version: string;
            };
          };
        };
      };
    };
  };
  /** Get Journals */
  get_journals: {
    parameters: {
      query: {
        keyword: string | null;
        links: string[] | null;
        page: number | null;
        size: number | null;
        tags: string[] | null;
      };
    };
    responses: {
      /** @description default return */
      200: {
        content: {
          'application/json': {
            data: {
              current_page: number;
              page_size: number;
              records: (
                | {
                    /** Format: date-time */
                    datetime: string;
                    flag: string;
                    /** Format: uuid */
                    id: string;
                    is_balanced: boolean;
                    links: string[];
                    metas: {
                      key: string;
                      value: string;
                    }[];
                    narration?: string | null;
                    payee: string;
                    postings: {
                      account: string;
                      account_after_commodity: string;
                      account_after_number: string;
                      account_before_commodity: string;
                      account_before_number: string;
                      cost_commodity?: string | null;
                      cost_number?: string | null;
                      inferred_unit_commodity: string;
                      inferred_unit_number: string;
                      unit_commodity?: string | null;
                      unit_number?: string | null;
                    }[];
                    sequence: number;
                    tags: string[];
                    /** @enum {string} */
                    type: 'Transaction';
                  }
                | {
                    /** Format: date-time */
                    datetime: string;
                    /** Format: uuid */
                    id: string;
                    narration?: string | null;
                    payee: string;
                    postings: {
                      account: string;
                      account_after_commodity: string;
                      account_after_number: string;
                      account_before_commodity: string;
                      account_before_number: string;
                      cost_commodity?: string | null;
                      cost_number?: string | null;
                      inferred_unit_commodity: string;
                      inferred_unit_number: string;
                      unit_commodity?: string | null;
                      unit_number?: string | null;
                    }[];
                    sequence: number;
                    /** @enum {string} */
                    type: 'BalanceCheck';
                    type_: string;
                  }
                | {
                    /** Format: date-time */
                    datetime: string;
                    /** Format: uuid */
                    id: string;
                    narration?: string | null;
                    payee: string;
                    postings: {
                      account: string;
                      account_after_commodity: string;
                      account_after_number: string;
                      account_before_commodity: string;
                      account_before_number: string;
                      cost_commodity?: string | null;
                      cost_number?: string | null;
                      inferred_unit_commodity: string;
                      inferred_unit_number: string;
                      unit_commodity?: string | null;
                      unit_number?: string | null;
                    }[];
                    sequence: number;
                    /** @enum {string} */
                    type: 'BalancePad';
                    type_: string;
                  }
              )[];
              total_count: number;
              total_page: number;
            };
          };
        };
      };
    };
  };
  /** Get All Options */
  get_all_options: {
    responses: {
      /** @description default return */
      200: {
        content: {
          'application/json': {
            data: {
              key: string;
              value: string;
            }[];
          };
        };
      };
    };
  };
  /** Plugin List */
  plugin_list: {
    responses: {
      /** @description default return */
      200: {
        content: {
          'application/json': {
            data: {
              name: string;
              plugin_type: ('Processor' | 'Mapper' | 'Router')[];
              version: string;
            }[];
          };
        };
      };
    };
  };
  /** Reload */
  reload: {
    responses: {
      /** @description default return */
      200: {
        content: {
          'application/json': {
            data: string;
          };
        };
      };
    };
  };
  /** Get Statistic Rank Detail By Account Type */
  get_statistic_rank_detail_by_account_type: {
    parameters: {
      query: {
        from: string;
        to: string;
      };
      path: {
        account_type: string;
      };
    };
    responses: {
      /** @description default return */
      200: {
        content: {
          'application/json': {
            data: {
              detail: {
                account: string;
                amount: {
                  /** @description the calculated amount */
                  calculated: {
                    /** @description the currency of the amount */
                    currency: string;
                    /** @description the number of the amount */
                    number: string;
                  };
                  /** @description the detail of the calculated amount */
                  detail: {
                    [key: string]: string;
                  };
                };
              }[];
              /** Format: date-time */
              from: string;
              /** Format: date-time */
              to: string;
              top_transactions: {
                account: string;
                account_after_commodity: string;
                account_after_number: string;
                /** Format: date-time */
                datetime: string;
                inferred_unit_commodity: string;
                inferred_unit_number: string;
                narration?: string | null;
                payee?: string | null;
                timestamp: number;
                trx_id: string;
              }[];
            };
          };
        };
      };
    };
  };
  /** Get Statistic Graph */
  get_statistic_graph: {
    parameters: {
      query: {
        from: string;
        to: string;
        interval: 'Day' | 'Week' | 'Month';
      };
    };
    responses: {
      /** @description default return */
      200: {
        content: {
          'application/json': {
            data: {
              balances: {
                [key: string]: {
                  /** @description the calculated amount */
                  calculated: {
                    /** @description the currency of the amount */
                    currency: string;
                    /** @description the number of the amount */
                    number: string;
                  };
                  /** @description the detail of the calculated amount */
                  detail: {
                    [key: string]: string;
                  };
                };
              };
              changes: {
                [key: string]: {
                  [key: string]: {
                    /** @description the calculated amount */
                    calculated: {
                      /** @description the currency of the amount */
                      currency: string;
                      /** @description the number of the amount */
                      number: string;
                    };
                    /** @description the detail of the calculated amount */
                    detail: {
                      [key: string]: string;
                    };
                  };
                };
              };
              /** Format: date-time */
              from: string;
              /** Format: date-time */
              to: string;
            };
          };
        };
      };
    };
  };
  /** Get Statistic Summary */
  get_statistic_summary: {
    parameters: {
      query: {
        from: string;
        to: string;
      };
    };
    responses: {
      /** @description default return */
      200: {
        content: {
          'application/json': {
            data: {
              balance: {
                /** @description the calculated amount */
                calculated: {
                  /** @description the currency of the amount */
                  currency: string;
                  /** @description the number of the amount */
                  number: string;
                };
                /** @description the detail of the calculated amount */
                detail: {
                  [key: string]: string;
                };
              };
              expense: {
                /** @description the calculated amount */
                calculated: {
                  /** @description the currency of the amount */
                  currency: string;
                  /** @description the number of the amount */
                  number: string;
                };
                /** @description the detail of the calculated amount */
                detail: {
                  [key: string]: string;
                };
              };
              from: string;
              income: {
                /** @description the calculated amount */
                calculated: {
                  /** @description the currency of the amount */
                  currency: string;
                  /** @description the number of the amount */
                  number: string;
                };
                /** @description the detail of the calculated amount */
                detail: {
                  [key: string]: string;
                };
              };
              liability: {
                /** @description the calculated amount */
                calculated: {
                  /** @description the currency of the amount */
                  currency: string;
                  /** @description the number of the amount */
                  number: string;
                };
                /** @description the detail of the calculated amount */
                detail: {
                  [key: string]: string;
                };
              };
              to: string;
              transaction_number: number;
            };
          };
        };
      };
    };
  };
  /** Create New Transaction */
  create_new_transaction: {
    requestBody: {
      content: {
        'application/json': {
          datetime: string;
          flag?: string | null;
          links: string[];
          metas: {
            key: string;
            value: string;
          }[];
          narration?: string | null;
          payee: string;
          postings: {
            account: string;
            unit?: {
              commodity: string;
              number: string;
            } | null;
          }[];
          tags: string[];
        };
      };
    };
    responses: {
      /** @description default return */
      200: {
        content: {
          'application/json': {
            data: string;
          };
        };
      };
    };
  };
  /** Update Single Transaction */
  update_single_transaction: {
    parameters: {
      path: {
        transaction_id: string;
      };
    };
    requestBody: {
      content: {
        'application/json': {
          datetime: string;
          flag?: string | null;
          links: string[];
          metas: {
            key: string;
            value: string;
          }[];
          narration?: string | null;
          payee: string;
          postings: {
            account: string;
            unit?: {
              commodity: string;
              number: string;
            } | null;
          }[];
          tags: string[];
        };
      };
    };
    responses: {
      /** @description default return */
      200: {
        content: {
          'application/json': {
            data?: Record<string, never>;
          };
        };
      };
    };
  };
  /** Upload Transaction Document */
  upload_transaction_document: {
    parameters: {
      path: {
        transaction_id: string;
      };
    };
    /** @description Multipart form data */
    requestBody: {
      content: {
        'multipart/form-data': unknown;
      };
    };
    responses: {
      /** @description default return */
      200: {
        content: {
          'application/json': {
            data: string;
          };
        };
      };
    };
  };
}
