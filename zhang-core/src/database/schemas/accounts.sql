
create table accounts
(
    date   datetime           not null,
    type   varchar            not null,
    name   varchar            not null
        primary key,
    status varchar            not null,
    alias  varchar
);
