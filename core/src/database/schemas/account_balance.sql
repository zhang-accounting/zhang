CREATE VIEW if not exists account_balance as
select transactions.datetime,
       account_max_datetime.account,
       account_after_number                         as balance_number,
       transaction_postings.account_after_commodity as balance_commodity
from transactions
         join transaction_postings on transactions.id = transaction_postings.trx_id

         join (select datetime, transactions.id, account, account_after_commodity
               from transaction_postings
                        join transactions on transactions.id = transaction_postings.trx_id
               group by account, account_after_commodity
               having max(sequence)) account_max_datetime
              on transactions.id = account_max_datetime.id and
                 transaction_postings.account = account_max_datetime.account
                  and transaction_postings.account_after_commodity = account_max_datetime.account_after_commodity