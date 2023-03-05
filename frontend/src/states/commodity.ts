import {atom} from 'jotai';
import {CommodityListItem} from "../rest-model";


export const commoditiesAtom = atom<{ [name: string]: CommodityListItem }>({})