export enum AccountStatus {
    Open = "Open", Close = "Close"
}

export interface Account {
    name: string,
    status: AccountStatus,
    commodities: { [commodity_name: string]: string }
}