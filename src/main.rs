use clang::{source, Clang, Entity, EntityKind, Index, TypeKind};
use std::{env::args_os, fmt::Debug, iter};
use tap::TapOps;

trait EntityVisitor: Debug {
    fn name(&self) -> Option<&str>;
    fn set_name(&mut self, new_name: String);
    fn entity_kind(&self) -> EntityKind;
    fn visit_entity(&mut self, current_entity: &Entity, parent_entity: &Entity);
}

trait TypeDeclaration: EntityVisitor {
    fn typedef_name(&self) -> Option<&str>;
    fn set_typedef_name(&mut self, new_typedef_name: String);
}

#[derive(Debug)]
struct SourceLocation {
    path: String,
    line_number: u32,
    column_number: u32,
}

impl SourceLocation {
    fn new(loc: (String, u32, u32)) -> Self {
        Self {
            path: loc.0,
            line_number: loc.1,
            column_number: loc.2,
        }
    }

    fn from_clang(loc: &source::SourceLocation) -> Self {
        Self::new(loc.get_presumed_location())
    }
}

#[derive(Debug)]
struct Type {
    type_kind: TypeKind,
    type_name: String,
    pointee_type: Option<Box<Type>>,
}

impl Type {
    fn new(kind: TypeKind, name: String, pointee_type: Option<Box<Type>>) -> Self {
        Self {
            type_kind: kind,
            type_name: name,
            pointee_type,
        }
    }

    fn from_clang(t: &clang::Type) -> Self {
        Self::new(
            t.get_kind(),
            t.get_display_name(),
            t.get_pointee_type()
                .map(|pt| Box::new(Self::from_clang(&pt))),
        )
    }
}

#[derive(Debug)]
struct SourceFile {
    path: String,
    type_declares: Vec<Box<dyn TypeDeclaration>>,
    function_declares: Vec<FunctionDeclare>,
}

impl SourceFile {
    fn new(path: String) -> Self {
        SourceFile {
            path,
            type_declares: Vec::new(),
            function_declares: Vec::new(),
        }
    }

    fn parse_type_declare(
        current_entity: &Entity,
        parent_entity: &Entity,
        declares: &mut Vec<Box<dyn TypeDeclaration>>,
    ) {
        match current_entity.get_kind() {
            EntityKind::EnumDecl => {
                if let Some(name) = current_entity.get_name() {
                    declares.push(Box::new(EnumDeclare::new(Some(name), None)).tap(
                        |enum_declare| {
                            enum_declare.visit_entity(current_entity, parent_entity);
                        },
                    ));
                }
            }
            EntityKind::StructDecl => {
                if let Some(name) = current_entity.get_name() {
                    declares.push(Box::new(StructDeclare::new(Some(name), None)).tap(
                        |struct_declare| {
                            struct_declare.visit_entity(current_entity, parent_entity);
                        },
                    ));
                }
            }
            EntityKind::UnionDecl => {
                if let Some(name) = current_entity.get_name() {
                    declares.push(Box::new(UnionDeclare::new(Some(name), None)).tap(
                        |union_declare| {
                            union_declare.visit_entity(current_entity, parent_entity);
                        },
                    ));
                }
            }
            EntityKind::TypedefDecl => {
                if let Some(declaration_entity) = current_entity
                    .get_typedef_underlying_type()
                    .and_then(|t| t.get_declaration())
                {
                    if let Some(declare) = declares
                        .iter_mut()
                        .filter(|declare| {
                            if let (Some(name), Some(typedef_name)) =
                                (declare.name(), current_entity.get_name())
                            {
                                return name == typedef_name;
                            }
                            false
                        })
                        .next()
                    {
                        if let Some(typedef_name) = current_entity.get_name() {
                            declare.set_typedef_name(typedef_name);
                        }
                    } else {
                        let declare: Box<dyn TypeDeclaration> = match declaration_entity.get_kind()
                        {
                            EntityKind::EnumDecl => Box::new(EnumDeclare::new(
                                None,
                                current_entity.get_name(),
                            ))
                            .tap(|enum_declare| {
                                enum_declare.visit_entity(&declaration_entity, parent_entity);
                            }),
                            EntityKind::StructDecl => Box::new(StructDeclare::new(
                                None,
                                current_entity.get_name(),
                            ))
                            .tap(|struct_declare| {
                                struct_declare.visit_entity(&declaration_entity, parent_entity);
                            }),
                            EntityKind::UnionDecl => Box::new(UnionDeclare::new(
                                None,
                                current_entity.get_name(),
                            ))
                            .tap(|union_declare| {
                                union_declare.visit_entity(&declaration_entity, parent_entity);
                            }),
                            _ => panic!(
                                "Unexpected typedef declaration entity: {:?}",
                                declaration_entity
                            ),
                        };
                        declares.push(declare);
                    }
                }
            }
            _ => panic!("Unexpected type entity: {:?}", current_entity),
        }
    }
}

impl EntityVisitor for SourceFile {
    #[inline]
    fn name(&self) -> Option<&str> {
        Some(self.path.as_ref())
    }

    #[inline]
    fn set_name(&mut self, new_name: String) {
        self.path = new_name;
    }

    #[inline]
    fn entity_kind(&self) -> EntityKind {
        EntityKind::TranslationUnit
    }

    fn visit_entity(&mut self, current_entity: &Entity, _: &Entity) {
        assert_eq!(current_entity.get_kind(), self.entity_kind());
        let children = current_entity.get_children();
        for next_entity in children.iter().filter(|entity| entity.is_in_main_file()) {
            match next_entity.get_kind() {
                EntityKind::EnumDecl | EntityKind::StructDecl | EntityKind::TypedefDecl => {
                    Self::parse_type_declare(next_entity, current_entity, &mut self.type_declares);
                }
                EntityKind::FunctionDecl => {
                    if let Some(function_name) = next_entity.get_name() {
                        self.function_declares
                            .push(FunctionDeclare::new(function_name).tap(|function_declare| {
                                function_declare.visit_entity(next_entity, current_entity);
                            }));
                    } else {
                        panic!("Unnamed function was declared")
                    }
                }
                _ => panic!("Unexpected entity: {:?}", next_entity),
            }
        }
    }
}

#[derive(Debug)]
struct EnumConstantValue {
    signed: i64,
    unsigned: u64,
}

#[derive(Debug)]
struct EnumConstantDeclare {
    name: String,
    location: Option<SourceLocation>,
    constant_value: Option<EnumConstantValue>,
}

impl EnumConstantDeclare {
    fn new(name: String) -> Self {
        Self {
            name,
            location: None,
            constant_value: None,
        }
    }
}

impl EntityVisitor for EnumConstantDeclare {
    #[inline]
    fn name(&self) -> Option<&str> {
        Some(self.name.as_ref())
    }

    #[inline]
    fn set_name(&mut self, new_name: String) {
        self.name = new_name;
    }

    #[inline]
    fn entity_kind(&self) -> EntityKind {
        EntityKind::EnumConstantDecl
    }

    fn visit_entity(&mut self, current_entity: &Entity, _: &Entity) {
        assert_eq!(current_entity.get_kind(), self.entity_kind());
        if let Some((signed_value, unsigned_value)) = current_entity.get_enum_constant_value() {
            self.constant_value = Some(EnumConstantValue {
                unsigned: unsigned_value,
                signed: signed_value,
            });
        }
        self.location = current_entity
            .get_location()
            .map(|source_location| SourceLocation::from_clang(&source_location));
    }
}

#[derive(Debug)]
struct EnumDeclare {
    enum_name: Option<String>,
    typedef_name: Option<String>,
    constants: Vec<EnumConstantDeclare>,
    enum_type: Option<Type>,
    location: Option<SourceLocation>,
}

impl EnumDeclare {
    fn new(enum_name: Option<String>, typedef_name: Option<String>) -> Self {
        Self {
            enum_name,
            typedef_name,
            constants: Vec::new(),
            enum_type: None,
            location: None,
        }
    }
}

impl EntityVisitor for EnumDeclare {
    #[inline]
    fn name(&self) -> Option<&str> {
        self.enum_name.as_ref().map(|s| s.as_ref())
    }

    fn set_name(&mut self, new_name: String) {
        self.enum_name = Some(new_name);
    }

    #[inline]
    fn entity_kind(&self) -> EntityKind {
        EntityKind::EnumDecl
    }

    fn visit_entity(&mut self, current_entity: &Entity, _: &Entity) {
        assert_eq!(current_entity.get_kind(), self.entity_kind());
        self.enum_type = current_entity
            .get_enum_underlying_type()
            .map(|enum_type| Type::from_clang(&enum_type));
        self.location = current_entity
            .get_location()
            .map(|source_location| SourceLocation::from_clang(&source_location));
        let children = current_entity.get_children();
        for child_entity in children.iter() {
            let name = child_entity.get_name().unwrap();
            self.constants
                .push(EnumConstantDeclare::new(name).tap(|enum_constant_declare| {
                    enum_constant_declare.visit_entity(child_entity, current_entity);
                }));
        }
    }
}

impl TypeDeclaration for EnumDeclare {
    #[inline]
    fn typedef_name(&self) -> Option<&str> {
        self.typedef_name.as_ref().map(|n| n.as_ref())
    }

    #[inline]
    fn set_typedef_name(&mut self, new_typedef_name: String) {
        self.typedef_name = Some(new_typedef_name);
    }
}

#[derive(Debug)]
struct FieldDeclare {
    name: Option<String>,
    field_type: Option<Type>,
    location: Option<SourceLocation>,
}

impl FieldDeclare {
    fn new(name: Option<String>) -> Self {
        Self {
            name,
            field_type: None,
            location: None,
        }
    }
}

impl EntityVisitor for FieldDeclare {
    #[inline]
    fn name(&self) -> Option<&str> {
        self.name.as_ref().map(|n| n.as_ref())
    }

    fn set_name(&mut self, new_name: String) {
        self.name = Some(new_name);
    }

    #[inline]
    fn entity_kind(&self) -> EntityKind {
        EntityKind::FieldDecl
    }

    fn visit_entity(&mut self, current_entity: &Entity, _: &Entity) {
        assert_eq!(current_entity.get_kind(), self.entity_kind());
        self.field_type = current_entity
            .get_type()
            .map(|field_type| Type::from_clang(&field_type));
        self.location = current_entity
            .get_location()
            .map(|source_location| SourceLocation::from_clang(&source_location));
    }
}

#[derive(Debug)]
struct StructDeclare {
    struct_name: Option<String>,
    typedef_name: Option<String>,
    fields: Vec<Box<dyn EntityVisitor>>,
    location: Option<SourceLocation>,
}

impl StructDeclare {
    fn new(struct_name: Option<String>, typedef_name: Option<String>) -> Self {
        Self {
            struct_name,
            typedef_name,
            fields: Vec::new(),
            location: None,
        }
    }
}

impl EntityVisitor for StructDeclare {
    #[inline]
    fn name(&self) -> Option<&str> {
        self.struct_name.as_ref().map(|n| n.as_ref())
    }

    fn set_name(&mut self, new_name: String) {
        self.struct_name = Some(new_name);
    }

    #[inline]
    fn entity_kind(&self) -> EntityKind {
        EntityKind::StructDecl
    }

    fn visit_entity(&mut self, current_entity: &Entity, _: &Entity) {
        assert_eq!(current_entity.get_kind(), self.entity_kind());
        self.location = current_entity
            .get_location()
            .map(|source_location| SourceLocation::from_clang(&source_location));
        let children = current_entity.get_children();
        for child_entity in children.iter() {
            match child_entity.get_kind() {
                EntityKind::FieldDecl => {
                    self.fields
                        .push(Box::new(FieldDeclare::new(child_entity.get_name())).tap(
                            |field_declare| {
                                field_declare.visit_entity(child_entity, current_entity);
                            },
                        ));
                }
                EntityKind::UnionDecl => {
                    self.fields.push(
                        Box::new(UnionDeclare::new(child_entity.get_name(), None)).tap(
                            |union_declare| {
                                union_declare.visit_entity(child_entity, current_entity);
                            },
                        ),
                    );
                }
                _ => panic!("Unexpected entity: {:?}"),
            }
        }
    }
}

impl TypeDeclaration for StructDeclare {
    #[inline]
    fn typedef_name(&self) -> Option<&str> {
        self.typedef_name.as_ref().map(|n| n.as_ref())
    }

    #[inline]
    fn set_typedef_name(&mut self, new_typedef_name: String) {
        self.typedef_name = Some(new_typedef_name);
    }
}

#[derive(Debug)]
struct UnionDeclare {
    union_name: Option<String>,
    typedef_name: Option<String>,
    fields: Vec<Box<dyn EntityVisitor>>,
    location: Option<SourceLocation>,
}

impl UnionDeclare {
    fn new(union_name: Option<String>, typedef_name: Option<String>) -> Self {
        Self {
            union_name,
            typedef_name,
            fields: Vec::new(),
            location: None,
        }
    }
}

impl EntityVisitor for UnionDeclare {
    #[inline]
    fn name(&self) -> Option<&str> {
        self.union_name.as_ref().map(|n| n.as_ref())
    }

    #[inline]
    fn set_name(&mut self, new_name: String) {
        self.union_name = Some(new_name);
    }

    #[inline]
    fn entity_kind(&self) -> EntityKind {
        EntityKind::UnionDecl
    }

    fn visit_entity(&mut self, current_entity: &Entity, _: &Entity) {
        assert_eq!(current_entity.get_kind(), self.entity_kind());
        self.location = current_entity
            .get_location()
            .map(|source_location| SourceLocation::from_clang(&source_location));
        let children = current_entity.get_children();
        for child_entity in children.iter() {
            self.fields
                .push(
                    Box::new(FieldDeclare::new(child_entity.get_name())).tap(|field_declare| {
                        field_declare.visit_entity(child_entity, current_entity);
                    }),
                );
        }
    }
}

impl TypeDeclaration for UnionDeclare {
    #[inline]
    fn typedef_name(&self) -> Option<&str> {
        self.typedef_name.as_ref().map(|n| n.as_ref())
    }

    #[inline]
    fn set_typedef_name(&mut self, new_typedef_name: String) {
        self.typedef_name = Some(new_typedef_name);
    }
}

#[derive(Debug)]
struct FunctionDeclare {
    function_name: String,
    return_type: Option<Type>,
    parameters: Vec<ParameterDeclare>,
    location: Option<SourceLocation>,
}

impl FunctionDeclare {
    fn new(function_name: String) -> Self {
        Self {
            function_name,
            return_type: None,
            parameters: Vec::new(),
            location: None,
        }
    }
}

impl EntityVisitor for FunctionDeclare {
    #[inline]
    fn name(&self) -> Option<&str> {
        Some(self.function_name.as_str())
    }

    #[inline]
    fn set_name(&mut self, new_function_name: String) {
        self.function_name = new_function_name;
    }

    #[inline]
    fn entity_kind(&self) -> EntityKind {
        EntityKind::FunctionDecl
    }

    fn visit_entity(&mut self, current_entity: &Entity, _: &Entity) {
        assert_eq!(current_entity.get_kind(), self.entity_kind());
        self.location = current_entity
            .get_location()
            .map(|source_location| SourceLocation::from_clang(&source_location));
        self.return_type = current_entity
            .get_result_type()
            .map(|return_type| Type::from_clang(&return_type));
        if let Some(arguments) = current_entity.get_arguments() {
            for argument in arguments.iter() {
                self.parameters
                    .push(ParameterDeclare::new(argument.get_name().unwrap()).tap(
                        |param_declare| {
                            param_declare.visit_entity(argument, current_entity);
                        },
                    ));
            }
        }
    }
}

#[derive(Debug)]
struct ParameterDeclare {
    name: String,
    parameter_type: Option<Type>,
    location: Option<SourceLocation>,
}

impl ParameterDeclare {
    fn new(name: String) -> Self {
        Self {
            name,
            parameter_type: None,
            location: None,
        }
    }
}

impl EntityVisitor for ParameterDeclare {
    #[inline]
    fn name(&self) -> Option<&str> {
        Some(self.name.as_str())
    }

    fn set_name(&mut self, new_name: String) {
        self.name = new_name;
    }

    #[inline]
    fn entity_kind(&self) -> EntityKind {
        EntityKind::ParmDecl
    }

    fn visit_entity(&mut self, current_entity: &Entity, _: &Entity) {
        assert_eq!(current_entity.get_kind(), self.entity_kind());
        self.parameter_type = current_entity
            .get_type()
            .map(|parameter_type| Type::from_clang(&parameter_type));
        self.location = current_entity
            .get_location()
            .map(|source_location| SourceLocation::from_clang(&source_location));
    }
}

fn show_entity(entity: Entity, level: usize) {
    if entity.is_in_main_file() {
        let prefix_spaces = iter::repeat(" ").take(level * 4).collect::<String>();
        println!("{}{:?}", prefix_spaces, entity);
        for child in entity.get_children() {
            show_entity(child, level + 1);
        }
    }
}

fn main() {
    let cl = Clang::new().unwrap();
    let idx = Index::new(&cl, true, false);
    for file_path in args_os().skip(1) {
        let tu = idx.parser(file_path).parse().unwrap();
        let entity = tu.get_entity();
        show_entity(entity, 0);

        if let Some(name) = entity.get_name() {
            let mut source_file = SourceFile::new(name);
            source_file.visit_entity(&entity, &entity);
            println!("****** source_file: {:#?}", source_file);
        }
    }
}
