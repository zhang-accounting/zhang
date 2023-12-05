create table errors
(
    id         varchat              not null
        primary key
        unique,
    filename   varchar,
    span_start integer,
    span_end   integer,
    content    varchar              not null,
    error_type varchar              not null,
    metas      varchar default '{}' not null
);

