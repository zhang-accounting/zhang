create table if not exists commodities
(
    name      varchar not null
        constraint commodities_pk
            primary key,
    precision INTEGER,
    prefix    varchar,
    suffix    varchar,
    rounding  varchar
);
