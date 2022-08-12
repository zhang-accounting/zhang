import { ApolloClient, ApolloProvider, InMemoryCache } from "@apollo/client";
import {
  Box, BoxProps, ChakraProvider, CloseButton, Drawer,
  DrawerContent, Flex, FlexProps, Icon, IconButton, Link, Text, useColorModeValue, useDisclosure
} from '@chakra-ui/react';
import React, { ReactNode, ReactText } from "react";
import ReactDOM from "react-dom";
import { IconType } from "react-icons";
import { FiCompass, FiHome, FiMenu, FiSettings, FiStar, FiTrendingUp } from 'react-icons/fi';
import { BrowserRouter, Link as RouteLink } from "react-router-dom";
import App from "./App";
import NewTransactionButton from "./components/NewTransactionButton";
import StatisticBar from "./components/StatisticBar";
import { Chart, registerables } from 'chart.js';
// @ts-ignore
import {createUploadLink } from 'apollo-upload-client';
import { relayStylePagination } from "@apollo/client/utilities";
import './i18n'
import { useTranslation } from "react-i18next";
Chart.register(...registerables);


const client = new ApolloClient({

  link: createUploadLink({
    uri: '/graphql'
  }),
  cache: new InMemoryCache({
    typePolicies: {
      Query: {
        fields: {
          errors: relayStylePagination()
        }
      }
    }
  })
});

interface LinkItemProps {
  name: string;
  icon: IconType;
  uri: string;
}
const LinkItems: Array<LinkItemProps> = [
  { name: 'NAV_HOME', icon: FiHome, uri: "/" },
  { name: 'NAV_JOURNALS', icon: FiTrendingUp, uri: "/journals" },
  { name: 'NAV_ACCOUNTS', icon: FiCompass, uri: "/accounts" },
  { name: 'NAV_COMMDOITIES', icon: FiStar, uri: "/commodities" },
  { name: 'NAV_DOCUMENTS', icon: FiSettings, uri: "/documents" },
  { name: 'NAV_REPORT', icon: FiSettings, uri: "/report" },
  { name: 'NAV_LIABILITY', icon: FiSettings, uri: "/liability" },
  { name: 'NAV_RAW_EDITING', icon: FiSettings, uri: "/edit" },
  { name: 'NAV_SETTING', icon: FiSettings, uri: "/settings" },
];


function SidebarWithHeader({
  children,
}: {
  children: ReactNode;
}) {
  const { isOpen, onOpen, onClose } = useDisclosure();
  return (
    <Box minH="100vh">
      <SidebarContent
        onClose={() => onClose}
        display={{ base: 'none', md: 'block' }}
      />
      <Drawer
        autoFocus={false}
        isOpen={isOpen}
        placement="left"
        onClose={onClose}
        returnFocusOnClose={false}
        onOverlayClick={onClose}
        size="full">
        <DrawerContent>
          <SidebarContent onClose={onClose} />
        </DrawerContent>
      </Drawer>
      {/* mobilenav */}
      <MobileNav onOpen={onOpen} />
      <Box ml={{ base: 0, md: 60 }} p="4">
        {children}
      </Box>
    </Box>
  );
}

interface SidebarProps extends BoxProps {
  onClose: () => void;
}

const SidebarContent = ({ onClose, ...rest }: SidebarProps) => {
  const {t} = useTranslation();
  return (
    <Box
      transition="3s ease"
      bg={useColorModeValue('white', 'gray.900')}
      borderRight="1px"
      borderRightColor={useColorModeValue('gray.200', 'gray.700')}
      w={{ base: 'full', md: 60 }}
      pos="fixed"
      h="full"
      {...rest}>
      <Flex h="20" alignItems="center" mx="8" justifyContent="space-between">
        <Text fontSize="2xl" fontFamily="monospace" fontWeight="bold">
          账 Zhang
        </Text>
        <CloseButton display={{ base: 'flex', md: 'none' }} onClick={onClose} />
      </Flex>
        <NewTransactionButton />
      {LinkItems.map((link) => (
        <NavItem key={link.name} icon={link.icon} uri={link.uri}>
          {t(link.name)}
        </NavItem>
      ))}
    </Box>
  );
};

interface NavItemProps extends FlexProps {
  icon: IconType;
  uri: string;
  children: ReactText;
}
const NavItem = ({ icon, children, uri, ...rest }: NavItemProps) => {
  return (
    <Link as={RouteLink} to={uri} style={{ textDecoration: 'none' }} _focus={{ boxShadow: 'none' }}>
      <Flex
        align="center"
        paddingLeft={4}
        paddingRight={4}
        paddingTop={2}
        paddingBottom={2}
        marginTop={1}
        mx="4"
        borderRadius="3"
        role="group"
        cursor="pointer"
        _hover={{
          bg: 'cyan.400',
          color: 'white',
        }}
        {...rest}>
        {icon && (
          <Icon
            mr="4"
            fontSize="16"
            _groupHover={{
              color: 'white',
            }}
            as={icon}
          />
        )}
        {children}
      </Flex>
    </Link>
  );
};

interface MobileProps extends FlexProps {
  onOpen: () => void;
}
const MobileNav = ({ onOpen, ...rest }: MobileProps) => {
  return (
    <Flex
      ml={{ base: 0, md: 60 }}
      px={{ base: 4, md: 4 }}
      height="20"
      alignItems="center"
      bg={useColorModeValue('white', 'gray.900')}
      borderBottomWidth="1px"
      borderBottomColor={useColorModeValue('gray.200', 'gray.700')}
      // justifyContent={{ base: 'space-between', md: 'flex-end' }}
      {...rest}>
      <IconButton
        display={{ base: 'flex', md: 'none' }}
        onClick={onOpen}
        variant="outline"
        aria-label="open menu"
        icon={<FiMenu />}
      />

      <Text
        display={{ base: 'flex', md: 'none' }}
        fontSize="2xl"
        fontFamily="monospace"
        fontWeight="bold">
        账 Zhang
      </Text>
      <StatisticBar />

    </Flex>
  );
};
ReactDOM.render(
  <React.StrictMode>
    <ChakraProvider>
      <BrowserRouter>
        <ApolloProvider client={client}>
          <SidebarWithHeader>
            <App></App>
          </SidebarWithHeader>

        </ApolloProvider>

      </BrowserRouter>
    </ChakraProvider>

  </React.StrictMode>,
  document.getElementById("root")
);
