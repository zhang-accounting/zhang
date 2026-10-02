import type { QueryAmount, QueryInventory, QueryPosition } from '@/api/types';

/**
 * Adds thousands separators to an exact decimal string without converting it to a float.
 * Values that are not plain decimals (e.g. scientific notation) are returned as-is.
 */
export function formatDecimal(value: string): string {
  const match = /^([+-]?)(\d+)(\.\d+)?$/.exec(value);
  if (!match) return value;
  const [, sign, integer, fraction = ''] = match;
  return `${sign === '-' ? '-' : ''}${integer.replace(/\B(?=(\d{3})+(?!\d))/g, ',')}${fraction}`;
}

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
