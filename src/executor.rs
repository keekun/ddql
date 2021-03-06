use std::fmt;
use std::sync::Arc;
// use rusoto_core::{Region, CredentialsError, HttpDispatchError};
use rusoto_core::Region;
use rusoto_dynamodb::{DynamoDb, DynamoDbClient, AttributeValue, AttributeDefinition,
                      KeySchemaElement, TableDescription, DescribeTableInput, DescribeTableError,
                      ListTablesInput, ListTablesError, ScanInput, ScanError, QueryInput,
                      QueryError, GetItemInput, GetItemError, BatchGetItemInput, BatchGetItemError};
use ddql::Query;
use common::Literal;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use prettytable::Table;
use prettytable::row::Row;
use prettytable::cell::Cell;
use select;

pub struct Executor {
    pub client: Arc<DynamoDbClient>,
    tables: RefCell<HashMap<String, TableDesc>>,
}

#[derive(Debug)]
pub struct ExecuteResult {
    pub table: String,
    pub items: RefCell<Vec<ExecuteResultItem>>,
}

impl ExecuteResult {
    pub fn new(table: String) -> Self {
        ExecuteResult {
            table: table,
            items: RefCell::new(Vec::new()),
        }
    }

    pub fn add_item(&self, item: ExecuteResultItem) {
        self.items.borrow_mut().push(item);
    }

    pub fn add_attrs_row(&self, attrs: HashMap<String, AttributeValue>) {
        self.items.borrow_mut().push(From::from(attrs));
    }

    pub fn print_table(&self) {
        let mut table = Table::new();

        let mut headers: HashSet<String> = HashSet::new();
        println!("new table {}", self.items.borrow().as_slice().len());
        for s in self.items.borrow().as_slice() {
            for (k, _) in s.attrs.borrow().iter() {
                headers.insert(k.to_string());
            }
        }
        let headers_vec = headers.iter().map(|x| x.as_str()).collect::<Vec<&str>>();
        let title_row = Row::new(headers_vec.iter().map(|x| Cell::new(x)).collect::<Vec<Cell>>());
        table.set_titles(title_row);

        for s in self.items.borrow().as_slice() {
            let mut r = Row::new(vec![]);
            let attrs = s.attrs.borrow();
            for (i, h) in headers_vec.iter().enumerate() {
                if let Some(v) = attrs.get(&h.to_string()) {
                    r.insert_cell(i, Cell::new(format!("{}", v).as_str()));
                } else {
                    r.insert_cell(i, Cell::new("--"));
                }
            }
            table.add_row(r);
        }
        table.printstd();
    }
}

#[derive(Debug)]
pub struct ExecuteResultItem {
    pub attrs: RefCell<HashMap<String, AttrValue>>,
}

impl ExecuteResultItem {
    // pub fn new(attrs: HashMap<String, AttrValue>) -> Self {
    //     ExecuteResultItem { attrs: attrs }
    // }

    pub fn new() -> Self {
        ExecuteResultItem { attrs: RefCell::new(HashMap::new()) }
    }

    pub fn add_key_value(&self, key: String, value: AttrValue) {
        self.attrs.borrow_mut().insert(key, value);
    }
}

impl From<HashMap<String, AttributeValue>> for ExecuteResultItem {
    fn from(m: HashMap<String, AttributeValue>) -> Self {
        let attrs = m.into_iter().map(|(k, v)| (k, From::from(v))).collect();
        ExecuteResultItem { attrs: RefCell::new(attrs) }
    }
}


#[derive(Debug)]
pub enum ExecuteError {
    DynamoDBListTableError(ListTablesError),
    DynamoDBScanError(ScanError),
    DynamoDBDescribeTableError(DescribeTableError),
    InvalidQuery,
}

impl From<ListTablesError> for ExecuteError {
    fn from(error: ListTablesError) -> Self {
        ExecuteError::DynamoDBListTableError(error)
    }
}

impl From<ScanError> for ExecuteError {
    fn from(error: ScanError) -> Self {
        ExecuteError::DynamoDBScanError(error)
    }
}

impl From<DescribeTableError> for ExecuteError {
    fn from(error: DescribeTableError) -> Self {
        ExecuteError::DynamoDBDescribeTableError(error)
    }
}

impl Executor {
    pub fn new(c: DynamoDbClient) -> Self {
        Executor {
            client: Arc::new(c),
            tables: RefCell::new(HashMap::new()),
        }
    }

    pub fn execute(&self, q: Query) -> Result<ExecuteResult, ExecuteError> {
        match q {
            Query::ShowTables => self.execute_show_tables(),
            Query::Select(s) => self.execute_select(s),
            _ => Err(ExecuteError::InvalidQuery),
        }
    }

    pub fn execute_select(&self,
                          s: select::SelectStatement)
                          -> Result<ExecuteResult, ExecuteError> {
        // let select_input:  = Default::default();
        println!("{}", s);
        self.execute_scan(s)
    }

    fn execute_scan(&self, s: select::SelectStatement) -> Result<ExecuteResult, ExecuteError> {
        let table_name = s.from_clause.table;
        let mut scan_input: ScanInput = Default::default();

        let desc = try!(self.load_table_desc(table_name.clone()));
        println!("table describe: {:?}", desc);

        // setup table name
        scan_input.table_name = table_name.clone();

        // setup fields
        if let select::FieldExpression::Fields(attr_names) = s.fields {
            scan_input.attributes_to_get = Some(attr_names.into_iter().map(|n| n.name).collect());
        }

        let output = try!(self.client.scan(&scan_input).sync());
        let res = ExecuteResult::new(table_name);
        if let Some(items) = output.items {
            for attrs in items {
                res.add_attrs_row(attrs);
            }
        }
        Ok(res)
    }

    fn execute_query(&self, s: select::SelectStatement) -> Result<ExecuteResult, ExecuteError> {
        Err(ExecuteError::InvalidQuery)
    }

    fn execute_get_item(&self, s: select::SelectStatement) -> Result<ExecuteResult, ExecuteError> {
        Err(ExecuteError::InvalidQuery)
    }

    fn execute_batch_get_items(&self,
                               s: select::SelectStatement)
                               -> Result<ExecuteResult, ExecuteError> {
        Err(ExecuteError::InvalidQuery)
    }

    pub fn execute_show_tables(&self) -> Result<ExecuteResult, ExecuteError> {
        let list_tables_input: ListTablesInput = Default::default();
        let output = try!(self.client.list_tables(&list_tables_input).sync());
        let res = ExecuteResult::new("tables".to_string());
        if let Some(table_name_list) = output.table_names {
            for table_name in table_name_list {
                let item = ExecuteResultItem::new();
                item.add_key_value(String::from("name"), From::from(table_name));
                res.add_item(item);
            }
        }
        Ok(res)
    }

    pub fn load_table_desc(&self, table: String) -> Result<TableDesc, ExecuteError> {
        if let Some(d) = self.tables.borrow_mut().get(&table) {
            return Ok(d.clone());
        }
        let desc_table_input =
            DescribeTableInput { table_name: table.clone(), ..Default::default() };
        let output = try!(self.client.describe_table(&desc_table_input).sync());
        if let Some(desc) = &output.table {
            if let Some(d) = TableDesc::from_desc(desc.clone()) {
                self.tables.borrow_mut().insert(table, d.clone());
                return Ok(d);
            }
        }
        Err(ExecuteError::InvalidQuery)
    }
}

#[derive(Debug, Clone)]
pub struct TableDesc {
    pub desc: TableDescription,
    pub key_schema: KeySchema,
}

impl TableDesc {
    pub fn from_desc(desc: TableDescription) -> Option<Self> {
        match &desc.attribute_definitions {
            Some(ref ads) => {
                let key_attrs: HashMap<String, KeyDef> = ads.into_iter()
                    .filter_map(|k| KeyDef::from_attr_def(k.clone()))
                    .map(|k| (k.name.clone(), k))
                    .collect();

                let hash_key = desc.clone()
                    .key_schema
                    .and_then(|ks| {
                        ks.into_iter()
                            .filter(|ref k| k.key_type.as_str() == "HASH")
                            .map(|k| k.attribute_name)
                            .nth(0)
                    })
                    .and_then(|ref k| key_attrs.get(k));

                let range_key = desc.clone()
                    .key_schema
                    .and_then(|ks| {
                        ks.into_iter()
                            .filter(|ref k| k.key_type.as_str() == "RANGE")
                            .map(|k| k.attribute_name)
                            .nth(0)
                    })
                    .and_then(|k| key_attrs.get(&k));

                hash_key.and_then(|hk| {
                    Some(TableDesc {
                        desc: desc.clone(),
                        key_schema: KeySchema {
                            hash: hk.clone(),
                            range: range_key.map(|x| x.clone()),
                        },
                    })
                })
            }
            None => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct KeySchema {
    pub hash: KeyDef,
    pub range: Option<KeyDef>,
}

#[derive(Debug, Clone)]
pub struct KeyDef {
    pub name: String,
    pub attr_type: KeyAttrType,
}

impl KeyDef {
    pub fn from_attr_def(v: AttributeDefinition) -> Option<Self> {
        KeyAttrType::from_string(v.attribute_type.clone()).map(|t| {
            KeyDef {
                name: v.attribute_name,
                attr_type: t,
            }
        })
    }
}

#[derive(Debug, Clone)]
pub enum KeyAttrType {
    String,
    Number,
    Binary,
}

impl KeyAttrType {
    fn from_string(value: String) -> Option<Self> {
        match value.to_uppercase().as_str() {
            "S" => Some(KeyAttrType::String),
            "N" => Some(KeyAttrType::Number),
            "B" => Some(KeyAttrType::Binary),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub enum AttrType {
    Null,
    Boolean,
    String,
    Number,
    Binary,
    List,
    Map,
    NumberSet,
    StringSet,
    BinarySet,
}

impl AttrType {
    fn from_string(value: String) -> Option<Self> {
        match value.to_uppercase().as_str() {
            "NULL" => Some(AttrType::Null),
            "BOOL" => Some(AttrType::Boolean),
            "S" => Some(AttrType::String),
            "N" => Some(AttrType::Number),
            "B" => Some(AttrType::Binary),
            "L" => Some(AttrType::List),
            "M" => Some(AttrType::Map),
            "NS" => Some(AttrType::NumberSet),
            "SS" => Some(AttrType::StringSet),
            "BS" => Some(AttrType::BinarySet),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub struct AttrValue {
    pub value: AttributeValue,
}

impl From<AttributeValue> for AttrValue {
    fn from(value: AttributeValue) -> Self {
        AttrValue { value: value }
    }
}

impl From<String> for AttrValue {
    fn from(value: String) -> Self {
        From::from(AttributeValue { s: Some(value), ..Default::default() })
    }
}

impl fmt::Display for AttrValue {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self.to_literal() {
            Some(l) => write!(f, "{}", l),
            None => write!(f, "--EMPTY--"),
        }
    }
}

impl AttrValue {
    pub fn new(value: AttributeValue) -> Self {
        AttrValue { value: value }
    }

    pub fn to_literal(&self) -> Option<Literal> {
        if let Some(ref v) = self.value.null {
            return Some(Literal::Null);
        }
        if let Some(ref v) = self.value.s {
            return Some(Literal::String(v.clone()));
        }
        if let Some(ref v) = self.value.n {
            return Some(Literal::Number(v.clone()));
        }
        if let Some(ref v) = self.value.bool {
            return Some(Literal::Boolean(v.clone()));
        }
        if let Some(ref v) = self.value.b {
            return Some(Literal::Binary(v.clone()));
        }
        if let Some(ref v) = self.value.m {
            return Some(Literal::Map(v.into_iter()
                .filter_map(|(ak, av)| {
                    AttrValue::new(av.clone()).to_literal().and_then(|l| Some((ak.clone(), l)))
                })
                .collect::<HashMap<String, Literal>>()));
        }
        if let Some(ref v) = self.value.l {
            return Some(Literal::List(v.into_iter()
                .filter_map(|av| AttrValue::new(av.clone()).to_literal())
                .collect::<Vec<Literal>>()));
        }
        if let Some(ref v) = self.value.ss {
            return Some(Literal::StringSet(v.into_iter()
                .filter_map(|v| AttrValue::from_string(v.to_string()).to_literal())
                .collect::<Vec<Literal>>()));
        }
        if let Some(ref v) = self.value.ns {
            return Some(Literal::NumberSet(v.into_iter()
                .filter_map(|v| AttrValue::from_number(v.to_string()).to_literal())
                .collect::<Vec<Literal>>()));
        }
        if let Some(ref v) = self.value.bs {
            return Some(Literal::BinarySet(v.into_iter()
                .filter_map(|v| AttrValue::from_binary(v.to_vec()).to_literal())
                .collect::<Vec<Literal>>()));
        }
        None
    }

    pub fn from_number(value: String) -> Self {
        From::from(AttributeValue { n: Some(value), ..Default::default() })
    }

    pub fn from_binary(value: Vec<u8>) -> Self {
        From::from(AttributeValue { b: Some(value), ..Default::default() })
    }

    pub fn from_string(value: String) -> Self {
        From::from(AttributeValue { s: Some(value), ..Default::default() })
    }
}
