terraform {
  required_version = ">= 1.5"

  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = ">= 5.0"
    }
  }
}

locals {
  name_prefix = "${var.project_name}-${var.environment}"

  default_tags = merge(var.tags, {
    Project     = var.project_name
    Environment = var.environment
    ManagedBy   = "terraform"
  })
}

# ---------------------------------------------------------------------------
# VPC (uses existing or creates default subnets)
# ---------------------------------------------------------------------------

data "aws_vpc" "selected" {
  count = var.vpc_id != "" ? 1 : 0
  id    = var.vpc_id
}

data "aws_subnets" "private" {
  filter {
    name   = "vpc-id"
    values = [var.vpc_id != "" ? var.vpc_id : data.aws_vpc.default.id]
  }

  tags = {
    Tier = "private"
  }
}

data "aws_subnets" "public" {
  filter {
    name   = "vpc-id"
    values = [var.vpc_id != "" ? var.vpc_id : data.aws_vpc.default.id]
  }

  tags = {
    Tier = "public"
  }
}

data "aws_vpc" "default" {
  default = true
}

locals {
  vpc_id     = var.vpc_id != "" ? var.vpc_id : data.aws_vpc.default.id
  subnet_ids = length(data.aws_subnets.private.ids) > 0 ? data.aws_subnets.private.ids : data.aws_subnets.public.ids
}

# ---------------------------------------------------------------------------
# Security Groups
# ---------------------------------------------------------------------------

resource "aws_security_group" "alb" {
  name_prefix = "${local.name_prefix}-alb-"
  vpc_id      = local.vpc_id

  ingress {
    from_port   = 80
    to_port     = 80
    protocol    = "tcp"
    cidr_blocks = ["0.0.0.0/0"]
  }

  ingress {
    from_port   = 443
    to_port     = 443
    protocol    = "tcp"
    cidr_blocks = ["0.0.0.0/0"]
  }

  egress {
    from_port   = 0
    to_port     = 0
    protocol    = "-1"
    cidr_blocks = ["0.0.0.0/0"]
  }

  tags = local.default_tags
}

resource "aws_security_group" "ecs" {
  name_prefix = "${local.name_prefix}-ecs-"
  vpc_id      = local.vpc_id

  ingress {
    from_port       = 8080
    to_port         = 8082
    protocol        = "tcp"
    security_groups = [aws_security_group.alb.id]
  }

  egress {
    from_port   = 0
    to_port     = 0
    protocol    = "-1"
    cidr_blocks = ["0.0.0.0/0"]
  }

  tags = local.default_tags
}

resource "aws_security_group" "db" {
  name_prefix = "${local.name_prefix}-db-"
  vpc_id      = local.vpc_id

  ingress {
    from_port       = 5432
    to_port         = 5432
    protocol        = "tcp"
    security_groups = [aws_security_group.ecs.id]
  }

  tags = local.default_tags
}

# ---------------------------------------------------------------------------
# Aurora Serverless v2 (PostgreSQL)
# ---------------------------------------------------------------------------

resource "aws_rds_cluster" "db" {
  cluster_identifier     = "${local.name_prefix}-db"
  engine                 = "aurora-postgresql"
  engine_mode            = "provisioned"
  engine_version         = "16.1"
  database_name          = "agentdb"
  master_username        = "agentdb"
  manage_master_user_password = true
  vpc_security_group_ids = [aws_security_group.db.id]
  skip_final_snapshot    = var.environment != "production"

  serverlessv2_scaling_configuration {
    min_capacity = var.db_min_capacity
    max_capacity = var.db_max_capacity
  }

  tags = local.default_tags
}

resource "aws_rds_cluster_instance" "db" {
  identifier         = "${local.name_prefix}-db-1"
  cluster_identifier = aws_rds_cluster.db.id
  instance_class     = "db.serverless"
  engine             = aws_rds_cluster.db.engine
  engine_version     = aws_rds_cluster.db.engine_version

  tags = local.default_tags
}

# ---------------------------------------------------------------------------
# ECS Cluster + Fargate Service
# ---------------------------------------------------------------------------

resource "aws_ecs_cluster" "main" {
  name = local.name_prefix

  setting {
    name  = "containerInsights"
    value = "enabled"
  }

  tags = local.default_tags
}

resource "aws_cloudwatch_log_group" "ecs" {
  name              = "/ecs/${local.name_prefix}"
  retention_in_days = 30
  tags              = local.default_tags
}

resource "aws_iam_role" "ecs_task_execution" {
  name_prefix = "${local.name_prefix}-exec-"

  assume_role_policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Action    = "sts:AssumeRole"
      Effect    = "Allow"
      Principal = { Service = "ecs-tasks.amazonaws.com" }
    }]
  })

  tags = local.default_tags
}

resource "aws_iam_role_policy_attachment" "ecs_task_execution" {
  role       = aws_iam_role.ecs_task_execution.name
  policy_arn = "arn:aws:iam::aws:policy/service-role/AmazonECSTaskExecutionRolePolicy"
}

resource "aws_ecs_task_definition" "app" {
  family                   = local.name_prefix
  network_mode             = "awsvpc"
  requires_compatibilities = ["FARGATE"]
  cpu                      = var.container_cpu
  memory                   = var.container_memory
  execution_role_arn       = aws_iam_role.ecs_task_execution.arn

  container_definitions = jsonencode([{
    name  = "agentdb"
    image = var.container_image

    portMappings = [
      { containerPort = 8080, protocol = "tcp" },
      { containerPort = 8081, protocol = "tcp" },
      { containerPort = 8082, protocol = "tcp" },
    ]

    environment = [
      { name = "AGENTDB_PORT", value = "8080" },
      { name = "AGENTDB_ADMIN_TOKEN", value = var.admin_token },
      { name = "AGENTDB_DEV_MODE", value = "false" },
      { name = "DATABASE_URL", value = "postgresql://${aws_rds_cluster.db.master_username}@${aws_rds_cluster.db.endpoint}:5432/agentdb" },
    ]

    logConfiguration = {
      logDriver = "awslogs"
      options = {
        "awslogs-group"         = aws_cloudwatch_log_group.ecs.name
        "awslogs-region"        = var.aws_region
        "awslogs-stream-prefix" = "agentdb"
      }
    }

    healthCheck = {
      command     = ["CMD-SHELL", "curl -f http://localhost:8080/health || exit 1"]
      interval    = 30
      timeout     = 5
      retries     = 3
      startPeriod = 10
    }
  }])

  tags = local.default_tags
}

# ---------------------------------------------------------------------------
# ALB
# ---------------------------------------------------------------------------

resource "aws_lb" "main" {
  name_prefix        = "adb-"
  internal           = false
  load_balancer_type = "application"
  security_groups    = [aws_security_group.alb.id]
  subnets            = local.subnet_ids

  tags = local.default_tags
}

resource "aws_lb_target_group" "app" {
  name_prefix = "adb-"
  port        = 8080
  protocol    = "HTTP"
  vpc_id      = local.vpc_id
  target_type = "ip"

  health_check {
    path                = "/health"
    healthy_threshold   = 2
    unhealthy_threshold = 3
    interval            = 30
    timeout             = 5
  }

  tags = local.default_tags
}

resource "aws_lb_listener" "http" {
  load_balancer_arn = aws_lb.main.arn
  port              = 80
  protocol          = "HTTP"

  default_action {
    type             = "forward"
    target_group_arn = aws_lb_target_group.app.arn
  }
}

resource "aws_lb_listener" "https" {
  count = var.certificate_arn != "" ? 1 : 0

  load_balancer_arn = aws_lb.main.arn
  port              = 443
  protocol          = "HTTPS"
  ssl_policy        = "ELBSecurityPolicy-TLS13-1-2-2021-06"
  certificate_arn   = var.certificate_arn

  default_action {
    type             = "forward"
    target_group_arn = aws_lb_target_group.app.arn
  }
}

# ---------------------------------------------------------------------------
# ECS Service
# ---------------------------------------------------------------------------

resource "aws_ecs_service" "app" {
  name            = local.name_prefix
  cluster         = aws_ecs_cluster.main.id
  task_definition = aws_ecs_task_definition.app.arn
  desired_count   = var.desired_count
  launch_type     = "FARGATE"

  network_configuration {
    subnets         = local.subnet_ids
    security_groups = [aws_security_group.ecs.id]
  }

  load_balancer {
    target_group_arn = aws_lb_target_group.app.arn
    container_name   = "agentdb"
    container_port   = 8080
  }

  tags = local.default_tags
}
