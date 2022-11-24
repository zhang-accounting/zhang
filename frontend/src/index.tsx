import { ApolloClient, ApolloProvider, InMemoryCache } from '@apollo/client';
import { Chart, registerables } from 'chart.js';
import React from 'react';
import ReactDOM from 'react-dom';
import { BrowserRouter } from 'react-router-dom';
import App from './App';
// @ts-ignore
import { createUploadLink } from 'apollo-upload-client';
import { MantineProvider } from '@mantine/core';
import './i18n';
import axios from 'axios';
Chart.register(...registerables);
// @ts-ignore
export const fetcher = (...args) => fetch(...args).then((res) => res.json());

export const axiosInstance = axios.create({
  baseURL: 'http://localhost:8000',
  headers: {
    'Content-type': 'application/json',
  },
});

const client = new ApolloClient({
  link: createUploadLink({
    uri: '/graphql',
  }),
  cache: new InMemoryCache({
    typePolicies: {
      Query: {
        fields: {
          errors: {
            read: (existing) => {
              return existing;
            },
            merge: (exists, incoming, options) => {
              return {
                ...incoming,
              };
            },
          },
        },
      },
    },
  }),
});

ReactDOM.render(
  <React.StrictMode>
    <MantineProvider withGlobalStyles withNormalizeCSS>
      <BrowserRouter>
        <ApolloProvider client={client}>
          <App></App>
        </ApolloProvider>
      </BrowserRouter>
    </MantineProvider>
  </React.StrictMode>,
  document.getElementById('root'),
);
