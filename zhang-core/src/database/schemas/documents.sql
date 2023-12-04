
create table if not exists documents
(
    datetime  datetime not null,
    filename  varchar  not null,
    path      varchar  not null,
    extension varchar,
    account   varchar,
    trx_id    varchar
);
