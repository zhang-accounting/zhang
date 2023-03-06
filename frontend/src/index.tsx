import { Chart, registerables } from 'chart.js';
import React from 'react';
import ReactDOM from 'react-dom';
import { BrowserRouter } from 'react-router-dom';
import App from './App';
import { MantineProvider } from '@mantine/core';
import './i18n';
import axios from 'axios';
import { ModalsProvider } from "@mantine/modals";
import { DocumentPreviewModal } from "./components/modals/DocumentPreviewModal";

Chart.register(...registerables);
// @ts-ignore
export const fetcher = (...args) => axiosInstance.get(...args).then((res) => res.data.data);
const development: boolean = !process.env.NODE_ENV || process.env.NODE_ENV === 'development';


if(development) {
    console.log("zhang is running in development mode");
}
export const serverBaseUrl = development ? 'http://localhost:8000' : "";
export const axiosInstance = axios.create({
    baseURL: serverBaseUrl,
    headers: {
        'Content-type': 'application/json',
    },
});

ReactDOM.render(
    <React.StrictMode>
        <MantineProvider withGlobalStyles withNormalizeCSS
            theme={{
                "colors": {
                    "red": [
                        "#FAEBED",
                        "#F0C7CC",
                        "#E6A2AB",
                        "#DD7E8A",
                        "#D35A69",
                        "#CA3548",
                        "#A12B3A",
                        "#79202B",
                        "#51151D",
                        "#280B0E"
                    ],
                    "blue": [
                        "#EBF6FA",
                        "#C6E5F1",
                        "#A1D5E8",
                        "#7CC4DF",
                        "#58B4D5",
                        "#33A3CC",
                        "#2983A3",
                        "#1E627B",
                        "#144152",
                        "#0A2129"
                    ],
                    "yellow": [
                        "#FDF3E8",
                        "#F8DDBF",
                        "#F4C895",
                        "#EFB26C",
                        "#EB9D42",
                        "#E68719",
                        "#B86C14",
                        "#8A510F",
                        "#5C360A",
                        "#2E1B05"
                    ],
                    "cyan": [
                        "#E5F8FF",
                        "#B8EDFF",
                        "#8AE1FF",
                        "#5CD5FF",
                        "#2ECAFF",
                        "#00BEFF",
                        "#0098CC",
                        "#007299",
                        "#004C66",
                        "#002633"
                    ]
                }
            }}
        >
            <ModalsProvider modals={{ documentPreviewModal: DocumentPreviewModal }}>
                <BrowserRouter>
                    <App></App>
                </BrowserRouter>
            </ModalsProvider>
        </MantineProvider>
    </React.StrictMode>,
    document.getElementById('root'),
);
