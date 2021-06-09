import Head from 'next/head'
import Layout, {siteTitle} from '../components/layout'
import utilStyles from '../styles/utils.module.css'
import Link from 'next/link'
import Date from '../components/date'
import {useEffect, useState} from "react";
import dynamic from "next/dynamic";
// import * as avaro from 'avaro-wasm';


const DynamicReactJson = dynamic(import('react-json-view'), {ssr: false});

const parse = async (content) => {
    const promise = await import("avaro-wasm/avaro_wasm");
    return promise.parse(content)
}


export default function Home({allPostsData}) {
    const [source, setSource] = useState("");
    const [target, setTarget] = useState([]);

    useEffect(() => {
        (async () => {
            const newVar = await parse(source);
            if (newVar.startsWith("[")) {
                setTarget(JSON.parse(newVar));
            } else {
                setTarget({error: newVar});
            }

        })()
    }, [source])
    return (
        <>
            <div className="page">
                <div>
                    <textarea name="" id="" cols="30" rows="10" value={source}
                              onChange={e => setSource(e.target.value)}/>
                </div>
                {/*<button onClick={parse}> click</button>*/}
                <div className="show">parse: <DynamicReactJson src={target}/></div>

            </div>
            <style jsx>{`
                    .page {
                    display: flex;
                    }
                
            `}</style>
        </>

    )
}
