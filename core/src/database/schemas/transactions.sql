create table transactions
(
    sequence    integer  not null
        primary key autoincrement
        unique,
    id          varchar  not null
        unique,
    datetime    datetime not null,
    type        varchar,
    payee       varchar,
    narration   varchar,
    source_file varchar  not null,
    span_start  integer  not null,
    span_end    integer  not null
);

create index transactions_id_index
    on transactions (id);