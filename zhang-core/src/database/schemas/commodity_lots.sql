create table if not exists commodity_lots
(
    commodity       varchar not null,
    datetime        datetime,
    amount          REAL,
    price_amount    REAL,
    price_commodity varchar,
    account         varchar
);
