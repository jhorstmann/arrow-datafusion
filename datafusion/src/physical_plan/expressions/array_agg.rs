// Licensed to the Apache Software Foundation (ASF) under one
// or more contributor license agreements.  See the NOTICE file
// distributed with this work for additional information
// regarding copyright ownership.  The ASF licenses this file
// to you under the Apache License, Version 2.0 (the
// "License"); you may not use this file except in compliance
// with the License.  You may obtain a copy of the License at
//
//   http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing,
// software distributed under the License is distributed on an
// "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied.  See the License for the
// specific language governing permissions and limitations
// under the License.

//! Defines physical expressions that can evaluated at runtime during query execution

use super::format_state_name;
use crate::error::{DataFusionError, Result};
use crate::physical_plan::{Accumulator, AggregateExpr, PhysicalExpr};
use crate::scalar::ScalarValue;
use arrow::array::ArrayRef;
use arrow::datatypes::{DataType, Field};
use std::any::Any;
use std::sync::Arc;

/// ARRAY_AGG aggregate expression
#[derive(Debug)]
pub struct ArrayAgg {
    name: String,
    input_data_type: DataType,
    expr: Arc<dyn PhysicalExpr>,
}

impl ArrayAgg {
    /// Create a new ArrayAgg aggregate function
    pub fn new(
        expr: Arc<dyn PhysicalExpr>,
        name: impl Into<String>,
        data_type: DataType,
    ) -> Self {
        Self {
            name: name.into(),
            expr,
            input_data_type: data_type,
        }
    }
}

impl AggregateExpr for ArrayAgg {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn field(&self) -> Result<Field> {
        Ok(Field::new(
            &self.name,
            DataType::List(Box::new(Field::new(
                "item",
                self.input_data_type.clone(),
                true,
            ))),
            false,
        ))
    }

    fn create_accumulator(&self) -> Result<Box<dyn Accumulator>> {
        Ok(Box::new(ArrayAggAccumulator::try_new(
            &self.input_data_type,
        )?))
    }

    fn state_fields(&self) -> Result<Vec<Field>> {
        Ok(vec![Field::new(
            &format_state_name(&self.name, "array_agg"),
            DataType::List(Box::new(Field::new(
                "item",
                self.input_data_type.clone(),
                true,
            ))),
            false,
        )])
    }

    fn expressions(&self) -> Vec<Arc<dyn PhysicalExpr>> {
        vec![self.expr.clone()]
    }

    fn name(&self) -> &str {
        &self.name
    }
}

#[derive(Debug)]
pub(crate) struct ArrayAggAccumulator {
    values: Vec<ScalarValue>,
    datatype: DataType,
}

impl ArrayAggAccumulator {
    /// new array_agg accumulator based on given item data type
    pub fn try_new(datatype: &DataType) -> Result<Self> {
        Ok(Self {
            values: vec![],
            datatype: datatype.clone(),
        })
    }
}

impl Accumulator for ArrayAggAccumulator {
    fn update_batch(&mut self, values: &[ArrayRef]) -> Result<()> {
        if values.is_empty() {
            return Ok(());
        };
        assert!(values.len() == 1, "array_agg can only take 1 param!");
        let arr = &values[0];
        (0..arr.len()).try_for_each(|index| {
            let scalar = ScalarValue::try_from_array(arr, index)?;
            self.values.push(scalar);
            Ok(())
        })
    }

    fn merge_batch(&mut self, states: &[ArrayRef]) -> Result<()> {
        if states.is_empty() {
            return Ok(());
        };
        assert!(states.len() == 1, "array_agg states must be singleton!");
        let arr = &states[0];
        (0..arr.len()).try_for_each(|index| {
            let scalar = ScalarValue::try_from_array(arr, index)?;
            if let ScalarValue::List(Some(values), _) = scalar {
                self.values.extend(*values);
                Ok(())
            } else {
                Err(DataFusionError::Internal(
                    "array_agg state must be list!".into(),
                ))
            }
        })
    }

    fn state(&self) -> Result<Vec<ScalarValue>> {
        Ok(vec![self.evaluate()?])
    }

    fn evaluate(&self) -> Result<ScalarValue> {
        Ok(ScalarValue::List(
            Some(Box::new(self.values.clone())),
            Box::new(self.datatype.clone()),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::field_util::SchemaExt;
    use crate::physical_plan::expressions::col;
    use crate::physical_plan::expressions::tests::aggregate;
    use crate::record_batch::RecordBatch;
    use crate::{error::Result, generic_test_op};
    use arrow::array::ArrayRef;
    use arrow::array::Int32Array;
    use arrow::datatypes::*;

    #[test]
    fn array_agg_i32() -> Result<()> {
        let a: ArrayRef = Arc::new(Int32Array::from_slice(vec![1, 2, 3, 4, 5]));

        let list = ScalarValue::List(
            Some(Box::new(vec![
                ScalarValue::Int32(Some(1)),
                ScalarValue::Int32(Some(2)),
                ScalarValue::Int32(Some(3)),
                ScalarValue::Int32(Some(4)),
                ScalarValue::Int32(Some(5)),
            ])),
            Box::new(DataType::Int32),
        );

        generic_test_op!(a, DataType::Int32, ArrayAgg, list, DataType::Int32)
    }

    #[test]
    fn array_agg_nested() -> Result<()> {
        let l1 = ScalarValue::List(
            Some(Box::new(vec![
                ScalarValue::List(
                    Some(Box::new(vec![
                        ScalarValue::from(1i32),
                        ScalarValue::from(2i32),
                        ScalarValue::from(3i32),
                    ])),
                    Box::new(DataType::Int32),
                ),
                ScalarValue::List(
                    Some(Box::new(vec![
                        ScalarValue::from(4i32),
                        ScalarValue::from(5i32),
                    ])),
                    Box::new(DataType::Int32),
                ),
            ])),
            Box::new(DataType::List(Box::new(Field::new(
                "item",
                DataType::Int32,
                true,
            )))),
        );

        let l2 = ScalarValue::List(
            Some(Box::new(vec![
                ScalarValue::List(
                    Some(Box::new(vec![ScalarValue::from(6i32)])),
                    Box::new(DataType::Int32),
                ),
                ScalarValue::List(
                    Some(Box::new(vec![
                        ScalarValue::from(7i32),
                        ScalarValue::from(8i32),
                    ])),
                    Box::new(DataType::Int32),
                ),
            ])),
            Box::new(DataType::List(Box::new(Field::new(
                "item",
                DataType::Int32,
                true,
            )))),
        );

        let l3 = ScalarValue::List(
            Some(Box::new(vec![ScalarValue::List(
                Some(Box::new(vec![ScalarValue::from(9i32)])),
                Box::new(DataType::Int32),
            )])),
            Box::new(DataType::List(Box::new(Field::new(
                "item",
                DataType::Int32,
                true,
            )))),
        );

        let list = ScalarValue::List(
            Some(Box::new(vec![l1.clone(), l2.clone(), l3.clone()])),
            Box::new(DataType::List(Box::new(Field::new(
                "item",
                DataType::Int32,
                true,
            )))),
        );

        let array: ArrayRef = ScalarValue::iter_to_array(vec![l1, l2, l3]).unwrap();

        generic_test_op!(
            array,
            DataType::List(Box::new(Field::new(
                "item",
                DataType::List(Box::new(Field::new("item", DataType::Int32, true,))),
                true,
            ))),
            ArrayAgg,
            list,
            DataType::List(Box::new(Field::new("item", DataType::Int32, true,)))
        )
    }
}
