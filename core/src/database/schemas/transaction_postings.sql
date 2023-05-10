create table if not exists transaction_postings
(
    trx_id                   varchar not null,
    account                  varchar not null,
    unit_number              REAL,
    unit_commodity           varchar,
    cost_number              REAL,
    cost_commodity           varchar,
    price_number             REAL,
    price_commodity          varchar,
    inferred_unit_number     REAL,
    inferred_unit_commodity  varchar,
    account_before_number    REAL,
    account_before_commodity varchar,
    account_after_number     REAL,
    account_after_commodity  varchar
);