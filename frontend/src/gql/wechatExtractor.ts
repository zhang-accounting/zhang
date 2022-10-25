import { gql } from '@apollo/client';

export const WECHAT_EXTRACTOR = gql`
  mutation WECHAT_EXTRACTOR($config: String!, $file: Upload!) {
    res: wechatExtractor(file: $file, config: $config)
  }
`;
