create table if not exists prices
(
    datetime         datetime not null,
    commodity        varchar  not null,
    amount           REAL  not null,
    target_commodity varchar  not null
);
