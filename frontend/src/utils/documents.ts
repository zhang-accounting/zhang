export const EXTENSIONS_SUPPORT_PREVIEW = ['png', 'jpg', 'jpeg', 'gif'];

export function isDocumentAnImage(path: string): boolean {
  const extension = path.split('.').pop()?.toLocaleLowerCase() || '';
  return EXTENSIONS_SUPPORT_PREVIEW.includes(extension);
}
