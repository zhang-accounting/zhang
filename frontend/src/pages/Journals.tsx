import {Pagination, Table, Title} from '@mantine/core';
import {useState} from 'react';
import useSWR from 'swr';
import {fetcher} from '..';
import {JournalItem, Pageable} from '../rest-model';
import {groupBy} from 'lodash';
import {format} from 'date-fns'
import TableViewJournalLine from '../components/journalLines/tableView/TableViewJournalLine';

function Journals() {

    const [page, setPage] = useState(1);
    const {data, error} = useSWR<Pageable<JournalItem>>(`/api/journals?page=${page}`, fetcher);

    if (error) return <div>failed to load</div>;
    if (!data) return <>loading</>;
    const {total_page, records, current_page} = data;
    const groupedRecords = groupBy(records, record => format(new Date(record.datetime), 'yyyy-MM-dd'));

    return (
        <>
            <Title order={2}>{records.length} Journals</Title>
            <Table verticalSpacing="xs" highlightOnHover>
                <thead>
                <tr>
                    <th>Date</th>
                    <th>Type</th>
                    <th>Payee</th>
                    <th>Narration</th>
                    <th>Amount</th>
                    <th>Operation</th>
                </tr>
                </thead>
                <tbody>
                {Object.entries(groupedRecords).map((entry) => {
                    return <>
                        {entry[1].map((journal) => (
                            <TableViewJournalLine key={journal.id} data={journal}/>
                        ))}
                    </>
                })}
                </tbody>
            </Table>
            <Pagination my="xs" total={total_page} page={current_page} onChange={setPage} position="center"/>

        </>
    );
}

export default Journals;
