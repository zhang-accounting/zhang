import { QueryAmount, QueryInventory, QueryPosition } from '@/api/types';

export function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

export function isAmount(value: unknown): value is QueryAmount {
  return isObject(value) && 'number' in value && 'currency' in value;
}

export function isPosition(value: unknown): value is QueryPosition {
  return isObject(value) && isAmount(value.units);
}

export function isInventory(value: unknown): value is QueryInventory {
  return isObject(value) && Array.isArray(value.positions);
}
