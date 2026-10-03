/**
 * The day to label a bucket of the report graph by: its first day, or the first day of the range when the bucket starts
 * before it (a week or a month the range starts in the middle of). The bucket's key stays its first day.
 */
export function labelDay(bucketStart: Date, rangeStart: Date): Date {
  return bucketStart.getTime() < rangeStart.getTime() ? rangeStart : bucketStart;
}
