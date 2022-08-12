import { Button, ButtonGroup, FormControl, FormLabel, Heading } from '@chakra-ui/react';
import { useTranslation } from 'react-i18next';

export default function Settings() {
    const { i18n } = useTranslation();
    const onLanguageChange = (lang: string) => {
        i18n.changeLanguage(lang);

    }

    return (
        <div>
            <Heading>Settings</Heading>

            <div>
                <FormControl>
                    <FormLabel>Lanauges</FormLabel>
                    <ButtonGroup size='sm' isAttached variant='outline'>
                        <Button onClick={() => onLanguageChange("zh")} disabled={i18n.language === 'zh'}>中文</Button>
                        <Button onClick={() => onLanguageChange("en")} disabled={i18n.language === 'en'}>English</Button>
                    </ButtonGroup>
                </FormControl>
            </div>
        </div>

    );
}